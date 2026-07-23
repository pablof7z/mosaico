use super::{pid_alive, revive_decision, session_still_live, DaemonState};
use crate::reconcile::{PresenceSnapshot, PublishReason, StatusEffect, StatusReconciler};
use crate::session_state::SessionState;
use crate::state::{AdmittedRuntimeFacts, RegisterSession, StopReason, Store};

#[test]
fn nonpositive_pid_is_never_alive() {
    // Defect #3: a synth ACP pid of 0 (`kill(0)` hits the caller's own group)
    // and negative pids (`kill(-n)` hits a whole group) must read as NOT live,
    // so a dead ACP session is never treated as an immortal ghost.
    assert!(!pid_alive(0));
    assert!(!pid_alive(-1));
}

#[test]
fn native_process_requires_a_live_pid() {
    assert!(!revive_decision(false, false, None));
}

#[test]
fn native_process_revives_on_pid_alone() {
    assert!(revive_decision(true, false, None));
}

#[test]
fn admitted_hosted_session_without_a_locator_never_uses_the_pid() {
    assert!(!revive_decision(true, true, None));
}

#[test]
fn live_hosted_endpoint_is_authoritative_without_a_pid() {
    assert!(revive_decision(false, true, Some(true)));
    assert!(revive_decision(true, true, Some(true)));
}

#[test]
fn dead_hosted_endpoint_is_not_revived_despite_a_live_pid() {
    // Guards against PID recycling: the process at `child_pid` is alive but
    // its supervisor socket is gone, so it is not our session.
    assert!(!revive_decision(true, true, Some(false)));
}

#[tokio::test]
async fn missing_hosted_locators_never_fall_back_to_a_live_pid() {
    let state = DaemonState::new_for_test().await;
    for transport in ["pty", "acp", "app-server"] {
        let pubkey = format!("pk-missing-{transport}");
        state
            .with_store(|store| {
                store.reserve_session_with_facts(
                    &crate::state::RegisterSession {
                        pubkey: pubkey.clone(),
                        observed_harness: "codex".into(),
                        agent_slug: "codex".into(),
                        channel_h: "root".into(),
                        child_pid: Some(std::process::id() as i32),
                        transcript_path: None,
                        now: 1,
                    },
                    &crate::state::AdmittedRuntimeFacts {
                        observed_harness: "codex".into(),
                        claimed_harness: String::new(),
                        bundle: format!("codex-{transport}"),
                        transport: transport.into(),
                        endpoint_provenance: "launch".into(),
                    },
                )
            })
            .unwrap();
        let session = state
            .with_store(|store| store.get_session(&pubkey))
            .unwrap()
            .unwrap();
        assert!(!session_still_live(&state, &session));
    }
}

#[test]
fn idle_eviction_and_exact_resume_reopen_presence_under_the_new_generation() {
    let store = Store::open_memory().unwrap();
    let register = |now| RegisterSession {
        pubkey: "pk-resume".into(),
        observed_harness: "codex".into(),
        agent_slug: "codex".into(),
        channel_h: "root".into(),
        child_pid: None,
        transcript_path: None,
        now,
    };
    let facts = AdmittedRuntimeFacts {
        observed_harness: "codex".into(),
        claimed_harness: String::new(),
        bundle: "codex-pty".into(),
        transport: "pty".into(),
        endpoint_provenance: "launch".into(),
    };
    let snapshot = |session: &crate::state::Session| PresenceSnapshot {
        host: "test-host".into(),
        slug: session.agent_slug.clone(),
        rel_cwd: ".".into(),
        dispatch_event: None,
        projection: crate::session_presence::publication(&store, session),
    };

    let first = store
        .reserve_session_with_facts(&register(10), &facts)
        .unwrap();
    let first_session = store.get_session("pk-resume").unwrap().unwrap();
    let mut presence = StatusReconciler::new(90, 30);
    presence.open("pk-resume", first, snapshot(&first_session), 10);
    assert!(store
        .mark_runtime_stopped_if_generation("pk-resume", first, StopReason::IdleEvicted, 20)
        .unwrap());
    presence.close("pk-resume", first, 20);

    let second = store
        .reserve_session_with_facts(&register(30), &facts)
        .unwrap();
    assert_eq!(second, first + 1);
    let resumed = store.get_session("pk-resume").unwrap().unwrap();
    let expected = crate::session_presence::publication(&store, &resumed).state;
    let opened = presence.open("pk-resume", second, snapshot(&resumed), 30);
    let published = opened.effects.iter().find_map(|effect| match effect {
        StatusEffect::Publish { status, reason } => Some((status, reason)),
        StatusEffect::Expire { .. } => None,
    });
    let (status, reason) = published.expect("new generation publishes a live presence lease");
    assert_eq!(*reason, PublishReason::Opened);
    assert_eq!(status.state, expected);
    assert_ne!(status.state, SessionState::Offline);
    assert!(presence.close("pk-resume", first, 31).effects.is_empty());
}
