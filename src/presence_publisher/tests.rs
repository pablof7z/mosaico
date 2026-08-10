use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Semaphore;
use tokio::time::{timeout, Duration};

use crate::domain::AgentRef;
use crate::reconcile::PublishReason;
use crate::session_state::SessionState;

type ObservedPublish = (String, u64, &'static str);
type BlockedPublisher = (
    PresencePublisher,
    Arc<Semaphore>,
    mpsc::UnboundedReceiver<ObservedPublish>,
);

fn renewal(pubkey: &str, revision: u64) -> StatusOutcome {
    StatusOutcome {
        effects: vec![StatusEffect::Publish {
            status: Status {
                agent: AgentRef::new(pubkey, "coder"),
                channels: vec!["room".into()],
                host: "laptop".into(),
                workspace: "mosaico".into(),
                branch: "master".into(),
                title: "Working".into(),
                activity: String::new(),
                state: SessionState::Working,
                state_since: 1,
                rel_cwd: ".".into(),
                expires_at: Some(90 + revision),
                dispatch_event: None,
            },
            reason: PublishReason::Renewed,
        }],
        revision,
        pubkey: Some(pubkey.into()),
    }
}

fn expiration(pubkey: &str, revision: u64) -> StatusOutcome {
    let StatusEffect::Publish { mut status, .. } = renewal(pubkey, revision).effects.remove(0)
    else {
        unreachable!("renewal fixture must publish")
    };
    status.state = SessionState::Offline;
    status.expires_at = Some(revision);
    StatusOutcome {
        effects: vec![StatusEffect::Expire { status }],
        revision,
        pubkey: Some(pubkey.into()),
    }
}

fn effect_name(job: &PublishJob) -> &'static str {
    match job.outcome.effects.first() {
        Some(StatusEffect::Publish { reason, .. }) => reason.as_str(),
        Some(StatusEffect::Expire { .. }) => "expire",
        None => "empty",
    }
}

fn blocked_publisher() -> BlockedPublisher {
    let pending = Arc::new(Mutex::new(PendingPublishJobs::default()));
    let (signal_tx, signal_rx) = mpsc::channel(PUBLISH_SIGNAL_CAPACITY);
    let publisher = PresencePublisher {
        signal_tx,
        pending: pending.clone(),
    };
    let gate = Arc::new(Semaphore::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let (observed_tx, observed_rx) = mpsc::unbounded_channel();
    spawn_publish_worker(signal_rx, pending, {
        let gate = gate.clone();
        move |job| {
            let gate = gate.clone();
            let calls = calls.clone();
            let observed_tx = observed_tx.clone();
            async move {
                let call = calls.fetch_add(1, Ordering::SeqCst);
                observed_tx
                    .send((
                        job.outcome.pubkey.clone().expect("job must name pubkey"),
                        job.outcome.revision,
                        effect_name(&job),
                    ))
                    .expect("test observer must remain open");
                if call == 0 {
                    gate.acquire()
                        .await
                        .expect("test gate must remain open")
                        .forget();
                }
            }
        }
    });
    (publisher, gate, observed_rx)
}

#[tokio::test]
async fn many_renewals_publish_only_the_running_job_and_latest_pending_state() {
    // Given the first publish is still running,
    let (publisher, gate, mut observed_rx) = blocked_publisher();
    let keys = Keys::generate();
    publisher.submit(renewal("pk-a", 1), &keys, "renewal");
    assert_eq!(
        timeout(Duration::from_secs(1), observed_rx.recv())
            .await
            .expect("the running job must start"),
        Some(("pk-a".into(), 1, "renewed"))
    );

    // When many newer full states arrive for the same pubkey,
    for revision in 2..=100 {
        publisher.submit(renewal("pk-a", revision), &keys, "renewal");
    }
    gate.add_permits(1);

    // Then the next executed job is the latest state, never stale revision 2.
    assert_eq!(
        timeout(Duration::from_secs(1), observed_rx.recv())
            .await
            .expect("the latest pending job must execute"),
        Some(("pk-a".into(), 100, "renewed"))
    );
    assert!(
        timeout(Duration::from_millis(50), observed_rx.recv())
            .await
            .is_err(),
        "intermediate renewals must not execute"
    );
}

#[tokio::test]
async fn expiration_replaces_a_queued_renewal_for_the_same_pubkey() {
    // Given one publish is running and a renewal is queued,
    let (publisher, gate, mut observed_rx) = blocked_publisher();
    let keys = Keys::generate();
    publisher.submit(renewal("pk-a", 1), &keys, "opened");
    timeout(Duration::from_secs(1), observed_rx.recv())
        .await
        .expect("the running job must start")
        .expect("the running job must be observed");
    publisher.submit(renewal("pk-a", 2), &keys, "renewal");

    // When an explicit expiration supersedes it,
    publisher.submit(expiration("pk-a", 3), &keys, "revoke");
    gate.add_permits(1);

    // Then only expiration runs after the in-flight job.
    assert_eq!(
        timeout(Duration::from_secs(1), observed_rx.recv())
            .await
            .expect("the expiration must execute"),
        Some(("pk-a".into(), 3, "expire"))
    );
    assert!(timeout(Duration::from_millis(50), observed_rx.recv())
        .await
        .is_err());
}

#[tokio::test]
async fn different_pubkeys_keep_first_pending_order_while_each_stays_latest() {
    // Given A is running,
    let (publisher, gate, mut observed_rx) = blocked_publisher();
    let keys = Keys::generate();
    publisher.submit(renewal("blocker", 1), &keys, "opened");
    timeout(Duration::from_secs(1), observed_rx.recv())
        .await
        .expect("the blocker must start")
        .expect("the blocker must be observed");

    // When B, A, a newer B, then C become pending,
    publisher.submit(renewal("pk-b", 2), &keys, "renewal");
    publisher.submit(renewal("pk-a", 3), &keys, "renewal");
    publisher.submit(renewal("pk-b", 4), &keys, "changed");
    publisher.submit(renewal("pk-c", 5), &keys, "renewal");
    gate.add_permits(1);

    // Then B keeps its first-pending position but publishes only revision 4.
    for expected in [
        ("pk-b".to_string(), 4, "renewed"),
        ("pk-a".to_string(), 3, "renewed"),
        ("pk-c".to_string(), 5, "renewed"),
    ] {
        assert_eq!(
            timeout(Duration::from_secs(1), observed_rx.recv())
                .await
                .expect("every distinct pending pubkey must execute"),
            Some(expected)
        );
    }
    assert!(timeout(Duration::from_millis(50), observed_rx.recv())
        .await
        .is_err());
}

#[test]
fn a_closed_worker_retains_no_unserviceable_presence_job() {
    let pending = Arc::new(Mutex::new(PendingPublishJobs::default()));
    let (signal_tx, signal_rx) = mpsc::channel(PUBLISH_SIGNAL_CAPACITY);
    drop(signal_rx);
    let publisher = PresencePublisher {
        signal_tx,
        pending: pending.clone(),
    };

    publisher.submit(renewal("pk-a", 1), &Keys::generate(), "renewal");

    assert_eq!(
        pending
            .lock()
            .expect("presence publish queue poisoned")
            .len(),
        0,
        "a closed worker cannot strand unserviceable pending jobs"
    );
}
