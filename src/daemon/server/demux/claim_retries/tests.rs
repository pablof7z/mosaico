use super::*;
use crate::fabric::nip29::orchestration::{build_add_agents_event, AddTarget};
use crate::fabric::nip29::session_dispatch::{build_session_dispatch_event, DispatchTarget};
use crate::state::{ORCHESTRATION_CLAIM_PREFIX, SESSION_DISPATCH_CLAIM_PREFIX};
use nostr::prelude::{Event, EventBuilder, FinalizeEvent, Keys, Kind, Tag};

/// Retry delay after attempt 1 and after attempt 2 (`state::event_claims`).
const DELAY_AFTER: [u64; 2] = [30, 60];
const PATIENCE: Duration = Duration::from_secs(30);

#[path = "tests/convergence.rs"]
mod convergence;
#[path = "tests/fences.rs"]
mod fences;
#[path = "tests/review.rs"]
mod review;
#[path = "tests/stalls.rs"]
mod stalls;

fn group_record(relay: &Keys, kind: u16, group: &str, member: &Keys) -> Event {
    EventBuilder::new(Kind::Custom(kind), "")
        .tags([
            Tag::custom("d", [group]),
            Tag::custom("name", [group]),
            Tag::custom("p", [member.public_key().to_hex()]),
        ])
        .finalize(relay)
        .unwrap()
}

#[derive(Clone)]
struct Fixture {
    event: Event,
    prefix: &'static str,
    groups: Vec<&'static str>,
    records: Vec<Event>,
}

impl Fixture {
    /// A session dispatch from a route-channel member, targeting `slug` here.
    fn session_dispatch(state: &Arc<DaemonState>, slug: &str) -> Self {
        let requester = Keys::generate();
        let relay = Keys::generate();
        let target = DispatchTarget {
            backend_pubkey: state.backend_pubkey(),
            slug: slug.into(),
            workspace: "workspace".into(),
            channels: vec!["route".into()],
        };
        let event = build_session_dispatch_event("route", &target, "dispatch")
            .unwrap()
            .tag(Tag::custom("h", ["route"]))
            .finalize(&requester)
            .unwrap();
        Self {
            event,
            prefix: SESSION_DISPATCH_CLAIM_PREFIX,
            groups: vec!["route"],
            records: vec![group_record(&relay, 39_002, "route", &requester)],
        }
    }

    /// An add-agents orchestration from a parent admin, spawning `slug` here.
    /// Without `child_in_view` the child's parent check reads the relay.
    fn orchestration(state: &Arc<DaemonState>, slug: &str, child_in_view: bool) -> Self {
        let admin = Keys::generate();
        let relay = Keys::generate();
        let adds = [AddTarget {
            backend_pubkey: state.backend_pubkey(),
            slug: slug.into(),
            session_pubkey: None,
        }];
        let event = build_add_agents_event("parent", "child", &adds, "add")
            .unwrap()
            .tag(Tag::custom("h", ["parent"]))
            .finalize(&admin)
            .unwrap();
        let mut records = vec![
            group_record(&relay, 39_000, "parent", &admin),
            group_record(&relay, 39_001, "parent", &admin),
        ];
        if child_in_view {
            records.push(group_record(&relay, 39_000, "child", &admin));
        }
        Self {
            event,
            prefix: ORCHESTRATION_CLAIM_PREFIX,
            groups: vec!["parent", "child"],
            records,
        }
    }

    fn show(&self, state: &Arc<DaemonState>) {
        let view = state.nip29_view();
        view.replace_group_scope(
            self.groups.iter().map(|group| group.to_string()).collect(),
            BTreeSet::new(),
        );
        let mut events = self.records.clone();
        events.push(self.event.clone());
        view.replace(events);
    }

    /// The same event with the group records of `kind` not yet observed.
    fn without_record(&self, kind: u16) -> Self {
        let mut fixture = self.clone();
        fixture
            .records
            .retain(|record| record.kind.as_u16() != kind);
        fixture
    }

    fn id(&self) -> String {
        self.event.id.to_hex()
    }

    fn requester(&self) -> String {
        self.event.pubkey.to_hex()
    }

    /// `(state, attempts, requester)` of the claim.
    fn claim(&self, state: &Arc<DaemonState>) -> Option<(String, u32, String)> {
        state.with_store(|store| store.event_claim_for_test(&self.id(), self.prefix))
    }

    fn state_and_attempts(&self, state: &Arc<DaemonState>) -> Option<(String, u32)> {
        self.claim(state)
            .map(|(claim_state, attempts, _)| (claim_state, attempts))
    }

    /// Poll for up to `budget` (virtual time once paused) for `expected`.
    async fn until(&self, state: &Arc<DaemonState>, expected: (&str, u32), budget: Duration) {
        let step = Duration::from_millis(10);
        for _ in 0..budget.as_millis() / step.as_millis() {
            if self.state_and_attempts(state) == Some((expected.0.to_string(), expected.1)) {
                return;
            }
            tokio::time::sleep(step).await;
        }
        panic!("claim never reached {expected:?}: {:?}", self.claim(state));
    }

    fn rewind(&self, state: &Arc<DaemonState>, secs: u64) {
        state.with_store(|store| store.rewind_event_claims_for_test(&self.id(), secs));
    }

    /// With no view wake, let the stored retry come due: freeze the demux
    /// timer, move the stored stamps back by the delay (wall-clock time
    /// passing), then advance the frozen timer by the same delay.
    async fn let_retry_come_due(&self, state: &Arc<DaemonState>) {
        tokio::time::pause();
        self.rewind(state, DELAY_AFTER[0]);
        tokio::time::advance(Duration::from_secs(DELAY_AFTER[0])).await;
    }
}

fn bind_workspace(state: &Arc<DaemonState>, channels: &[&str]) -> tempfile::TempDir {
    let workspace = tempfile::tempdir().unwrap();
    state.with_store(|store| {
        for channel in channels {
            crate::daemon::workspace_path::WorkspacePathResolver::new(store)
                .bind_root_path(channel, workspace.path(), now_secs())
                .unwrap();
        }
    });
    workspace
}

#[tokio::test]
async fn failed_session_dispatch_is_retried_with_no_other_traffic() {
    let state = DaemonState::new_for_test().await;
    let fixture = Fixture::session_dispatch(&state, "worker");
    fixture.show(&state);
    super::super::spawn_demux(state.clone());
    fixture.until(&state, ("pending", 1), PATIENCE).await;

    fixture.let_retry_come_due(&state).await;

    fixture.until(&state, ("pending", 2), PATIENCE).await;
}

#[tokio::test]
async fn failed_orchestration_target_is_retried_with_no_other_traffic() {
    let state = DaemonState::new_for_test().await;
    let fixture = Fixture::orchestration(&state, "worker", true);
    fixture.show(&state);
    super::super::spawn_demux(state.clone());
    fixture.until(&state, ("pending", 1), PATIENCE).await;

    fixture.let_retry_come_due(&state).await;

    fixture.until(&state, ("pending", 2), PATIENCE).await;
}

#[tokio::test]
async fn a_restart_inside_the_backoff_still_retries_once_due() {
    let state = DaemonState::new_for_test().await;
    let fixture = Fixture::session_dispatch(&state, "worker");
    fixture.show(&state);
    let key = format!(
        "{SESSION_DISPATCH_CLAIM_PREFIX}{}:worker",
        state.backend_pubkey()
    );
    // A previous daemon run claimed the dispatch and recorded one failure.
    state.with_store(|store| {
        assert!(store
            .claim_orchestration_target(&fixture.id(), &key, "admin", "route", "x", now_secs())
            .unwrap());
        store
            .retry_orchestration_target(&fixture.id(), &key, now_secs())
            .unwrap();
    });

    // The new demux's first pass is inside the backoff and cannot claim it.
    super::super::spawn_demux(state.clone());
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        fixture.state_and_attempts(&state),
        Some(("pending".to_string(), 1))
    );

    fixture.let_retry_come_due(&state).await;

    fixture.until(&state, ("pending", 2), PATIENCE).await;
}

#[tokio::test]
async fn an_unknown_agent_slug_fails_the_dispatch_permanently() {
    let state = DaemonState::new_for_test().await;
    let _workspace = bind_workspace(&state, &["workspace"]);
    let fixture = Fixture::session_dispatch(&state, "no-such-agent-anywhere");
    fixture.show(&state);
    super::super::spawn_demux(state.clone());

    fixture.until(&state, ("failed", 0), PATIENCE).await;
    let next = state.with_store(|store| store.next_claim_due_excluding(&BTreeSet::new()));
    assert_eq!(next.unwrap(), None);
}

#[tokio::test]
async fn an_unknown_agent_slug_fails_the_orchestration_target_permanently() {
    let state = DaemonState::new_for_test().await;
    let _workspace = bind_workspace(&state, &["parent", "child"]);
    let fixture = Fixture::orchestration(&state, "no-such-agent-anywhere", true);
    fixture.show(&state);
    super::super::spawn_demux(state.clone());

    fixture.until(&state, ("failed", 0), PATIENCE).await;
}

#[tokio::test]
async fn respawning_the_demux_aborts_the_previous_loop() {
    let state = DaemonState::new_for_test().await;
    super::super::spawn_demux(state.clone());
    let first = state.dedup.demux_task.lock().unwrap().clone().unwrap();

    super::super::spawn_demux(state.clone());

    for _ in 0..100 {
        if first.is_finished() {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(first.is_finished(), "the replaced demux must stop");
    let current = state.dedup.demux_task.lock().unwrap().clone().unwrap();
    assert!(!current.is_finished());
}

#[tokio::test]
async fn offline_mention_retries_ride_the_same_timer() {
    let state = DaemonState::new_for_test().await;
    let mentioned = Keys::generate().public_key().to_hex();
    state.with_store(|store| {
        assert!(store
            .claim_offline_mention("mention", &mentioned, "sender", "room", "body", now_secs())
            .unwrap());
        store
            .retry_offline_mention("mention", &mentioned, now_secs())
            .unwrap();
    });
    let mut driver = Driver::new();
    let pending =
        || state.with_store(|store| store.event_claim_for_test("mention", "offline-mention:"));
    assert!(driver.wait(&state).unwrap() > Instant::now());

    state.with_store(|store| store.rewind_event_claims_for_test("mention", DELAY_AFTER[0]));
    assert!(driver.wait(&state).unwrap() <= Instant::now());
    driver.redispatch_due(&state);

    assert_eq!(pending().unwrap().0, "processing");
}
