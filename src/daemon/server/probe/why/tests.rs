use crate::reconcile::{
    CoverageSnapshot, StatusReconciler, SubscriptionReconciler, TurnLifecycleReconciler,
    TurnProjectionSeed,
};
use std::collections::{BTreeMap, BTreeSet};

/// `status:` explain surfaces the labeled last command + its input cause.
#[test]
fn status_handle_explains_last_command() {
    let mut r = StatusReconciler::new(90, 30);
    r.on_session_started(
        "s1",
        "laptop",
        "coder",
        ".",
        BTreeSet::from(["room".to_string()]),
        true,
        "T",
        "",
        100,
    )
    .unwrap();
    r.on_distill("s1", "T", "compiling", 100).unwrap();

    let why = r.explain_status("s1").unwrap();
    assert_eq!(why.resource_key, "status/s1");
    assert_eq!(why.last_kind, "Replace");
    assert!(why.input_causes.iter().any(|l| l == "status/s1/activity"));
}

/// `sub:` explain surfaces owners + refcount + the labeled cause.
#[test]
fn sub_handle_explains_owners_and_refcount() {
    let mut r = SubscriptionReconciler::new().unwrap();
    let mut sessions = BTreeMap::new();
    sessions.insert("s1".to_string(), BTreeSet::from(["general".to_string()]));
    r.sync(&CoverageSnapshot {
        daemon_channels: BTreeSet::from(["general".to_string()]),
        addressed_pubkeys: BTreeSet::new(),
        archived_channels: BTreeSet::new(),
        sessions,
    })
    .unwrap();

    let why = r.explain_channel("general");
    assert_eq!(why.resource_key, "sub/h/general");
    assert_eq!(why.refcount, 2);
    assert_eq!(why.last_kind.as_deref(), Some("Open"));
}

#[test]
fn turn_handle_explains_projection_cause() {
    let mut r = TurnLifecycleReconciler::new();
    r.on_turn_started(
        TurnProjectionSeed {
            pubkey: "s1".into(),
            working: false,
            turn_started_at: 0,
            transcript_ref: None,
        },
        100,
        None,
    )
    .unwrap();

    let why = r.explain_turn("s1").unwrap();
    assert_eq!(why.resource_key, "turn_lifecycle/s1");
    assert_eq!(why.last_kind, "Open");
    assert!(why
        .input_causes
        .iter()
        .any(|l| l == "turn_lifecycle/s1/turn_started"));
}

#[test]
fn cursor_handle_explains_projection_cause() {
    let mut r = crate::reconcile::CursorReconciler::new();
    r.request(
        crate::reconcile::CursorSeed {
            pubkey: "s1".into(),
            seen_cursor: 10,
        },
        crate::reconcile::InputFact::TurnCheckRequested {
            pubkey: "s1".into(),
            observed_cursor: 10,
            working: true,
            at: 20,
        },
    )
    .unwrap();

    let why = r.explain_cursor("s1").unwrap();
    assert_eq!(why.resource_key, "cursor/s1");
    assert!(why
        .input_causes
        .iter()
        .any(|l| l == "cursor/s1/observed_cursor"));
}

#[test]
fn outbox_handle_explains_projection_cause() {
    let mut r = crate::reconcile::OutboxReconciler::new();
    r.drive(crate::reconcile::InputFact::OutboxEnqueueApplied {
        local_id: 7,
        event_id: "ev7".into(),
        event_hash: "sha256:event".into(),
        source_surface: "status".into(),
        source_ref: "status/s1#tx:1".into(),
        at: 100,
    })
    .unwrap();

    let why = r.explain_outbox(7).unwrap();
    assert_eq!(why.resource_key, "outbox/7");
    assert!(why.input_causes.iter().any(|l| l == "outbox/7/event_id"));
}

#[test]
fn session_watch_handle_explains_liveness_cause() {
    let mut r = crate::reconcile::Reconciler::new().unwrap();
    r.apply(&crate::reconcile::InputFact::SessionStarted {
        pubkey: "s1".into(),
        channel_h: Some("room".into()),
        pid: Some(42),
        at: 100,
    })
    .unwrap();

    let why = r.explain_watch("s1").unwrap();
    assert_eq!(why.resource_key, "session-watch/s1");
    assert_eq!(why.last_kind, "Open");
    assert!(why
        .input_causes
        .iter()
        .any(|l| l.starts_with("session_watch/")));
}
