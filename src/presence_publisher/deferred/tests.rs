use super::*;
use crate::presence_publisher::tests::renewal;
use crate::presence_publisher::{StatusOutcome, PUBLISH_SIGNAL_CAPACITY};
use crate::state::Store;
use nostr::key::Keys;
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
    let deferred = Arc::new(Mutex::new(DeferredJobs::default()));
    deferred.lock().unwrap().park(job(renewal("pk-a", 3)));
    let pending = Arc::new(Mutex::new(PendingPublishJobs::default()));
    let (signal_tx, mut signal_rx) = mpsc::channel(PUBLISH_SIGNAL_CAPACITY);
    spawn_requeue_on_group_change(
        Arc::new(RwLock::new(provider)),
        deferred.clone(),
        pending.clone(),
        signal_tx,
    );
    // Let the requeue task subscribe before the group state changes.
    tokio::task::yield_now().await;
    assert!(
        timeout(Duration::from_millis(200), signal_rx.recv())
            .await
            .is_err(),
        "nothing may be requeued before the group state changes"
    );
    assert_eq!(pending.lock().unwrap().len(), 0);

    view.replace(vec![EventBuilder::new(Kind::Custom(39_000), "")
        .tags([Tag::custom("d", ["room"]), Tag::custom("name", ["room"])])
        .finalize(&Keys::generate())
        .unwrap()]);

    timeout(Duration::from_secs(2), signal_rx.recv())
        .await
        .expect("a group change must requeue the parked status")
        .expect("signal");
    assert_eq!(pending.lock().unwrap().jobs["pk-a"].outcome.revision, 3);
    assert_eq!(deferred.lock().unwrap().len(), 0);
}
