use super::*;
use crate::presence_publisher::tests::renewal;
use crate::presence_publisher::StatusOutcome;
use crate::state::Store;
use nostr::key::Keys;
use std::sync::Mutex;
use std::time::Duration;
use tokio::time::timeout;

fn job(outcome: StatusOutcome) -> PublishJob {
    PublishJob {
        outcome,
        keys: Keys::generate(),
        trigger: "test".into(),
        confirmed_scope: None,
    }
}

/// A newer status that publishes through the normal queue retires an older
/// parked one, so the next group change cannot requeue stale presence over it.
#[test]
fn a_status_published_after_a_parked_one_retires_it() {
    let mut deferred = DeferredJobs::default();
    deferred.settle(job(renewal("pk-a", 1)), true);
    assert_eq!(deferred.len(), 1);

    deferred.settle(job(renewal("pk-a", 2)), false);

    let mut pending = PendingPublishJobs::default();
    assert_eq!(deferred.requeue_into(&mut pending), 0);
    assert_eq!(pending.len(), 0);
}

#[test]
fn a_deferred_status_is_requeued_unless_a_newer_state_is_queued() {
    let mut deferred = DeferredJobs::default();
    deferred.park(job(renewal("pk-a", 2)));
    deferred.park(job(renewal("pk-a", 1)));
    deferred.park(job(renewal("pk-b", 5)));
    let mut pending = PendingPublishJobs::default();
    pending.push(job(renewal("pk-b", 6)));

    assert_eq!(deferred.requeue_into(&mut pending), 1);
    assert_eq!(deferred.len(), 0);
    assert_eq!(pending.jobs["pk-a"].outcome.revision, 2);
    assert_eq!(pending.jobs["pk-b"].outcome.revision, 6);
}

/// The requeue is driven by the view's group-change wake: once the relay's
/// group records arrive (as after a daemon restart), a parked status is
/// queued again instead of waiting for the next lease renewal.
type Observed = tokio::sync::mpsc::UnboundedReceiver<u64>;

/// A publisher whose publications are recorded by revision and settled like
/// the real worker; the first one blocks until `gate` opens.
fn recording_publisher() -> (PresencePublisher, Arc<tokio::sync::Semaphore>, Observed) {
    let deferred = Arc::new(Mutex::new(DeferredJobs::default()));
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let first = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let (observed_tx, observed_rx) = tokio::sync::mpsc::unbounded_channel();
    let publisher = PresencePublisher::start(
        Arc::new(Mutex::new(PendingPublishJobs::default())),
        deferred.clone(),
        {
            let gate = gate.clone();
            Arc::new(move |job: PublishJob| {
                let gate = gate.clone();
                let first = first.clone();
                let deferred = deferred.clone();
                let observed_tx = observed_tx.clone();
                Box::pin(async move {
                    observed_tx.send(job.outcome.revision).unwrap();
                    if first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                        gate.acquire().await.unwrap().forget();
                    }
                    deferred.lock().unwrap().settle(job, false);
                })
            })
        },
    );
    (publisher, gate, observed_rx)
}

/// R1 is parked; R2 for the same pubkey is publishing. A requeue racing that
/// publication must wait for it (and the retirement of R1 it settles), so
/// stale R1 is never published over R2.
#[tokio::test]
async fn a_requeue_never_publishes_a_parked_status_over_a_newer_one_in_flight() {
    let (publisher, gate, mut observed) = recording_publisher();
    publisher
        .deferred
        .lock()
        .unwrap()
        .park(job(renewal("pk-a", 1)));
    publisher.submit(renewal("pk-a", 2), &Keys::generate(), "renewal", None);
    assert_eq!(
        timeout(Duration::from_secs(1), observed.recv())
            .await
            .unwrap(),
        Some(2)
    );

    let requeue = tokio::spawn({
        let publisher = publisher.clone();
        async move { publisher.requeue_deferred().await }
    });
    tokio::task::yield_now().await;
    assert!(publisher.lane.try_lock().is_err());
    assert!(!requeue.is_finished());
    gate.add_permits(1);

    assert!(
        !timeout(Duration::from_secs(1), requeue)
            .await
            .unwrap()
            .unwrap(),
        "the parked status must have been retired by the newer one"
    );
    assert_eq!(publisher.pending.lock().unwrap().len(), 0);
    assert!(observed.try_recv().is_err());
}

#[tokio::test]
async fn a_group_change_requeues_a_parked_status() {
    use crate::fabric::provider::Nip29Provider;
    use nostr::prelude::{EventBuilder, FinalizeEvent, Kind, Tag};

    let provider = Arc::new(Nip29Provider::new(
        Arc::new(crate::nostr_runtime::NostrRuntime::new("wss://relay.invalid").unwrap()),
        Arc::new(crate::fabric::nip29::view::Nip29ViewStore::default()),
        Arc::new(Mutex::new(Store::open_memory().unwrap())),
        Keys::generate().public_key(),
        None,
        Vec::new(),
    ));
    let view = provider.view.clone();
    view.replace_group_scope(
        std::collections::BTreeSet::from(["room".to_string()]),
        std::collections::BTreeSet::new(),
    );
    let (publisher, gate, mut observed) = recording_publisher();
    gate.add_permits(1);
    publisher
        .deferred
        .lock()
        .unwrap()
        .park(job(renewal("pk-a", 3)));
    spawn_requeue_on_group_change(Arc::new(RwLock::new(provider)), publisher.clone());
    // Let the requeue task subscribe before the group state changes.
    tokio::task::yield_now().await;
    assert!(
        timeout(Duration::from_millis(200), observed.recv())
            .await
            .is_err(),
        "nothing may be requeued before the group state changes"
    );

    view.replace(vec![EventBuilder::new(Kind::Custom(39_000), "")
        .tags([Tag::custom("d", ["room"]), Tag::custom("name", ["room"])])
        .finalize(&Keys::generate())
        .unwrap()]);

    assert_eq!(
        timeout(Duration::from_secs(2), observed.recv())
            .await
            .expect("a group change must requeue the parked status"),
        Some(3)
    );
    assert_eq!(publisher.deferred.lock().unwrap().len(), 0);
}
