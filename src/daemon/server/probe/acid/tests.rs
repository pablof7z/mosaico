use super::*;
use crate::reconcile::CoverageSnapshot;
use std::collections::{BTreeMap, BTreeSet};

#[tokio::test]
async fn status_acid_verifies_activity_cause_and_unrelated_hash() {
    let state = DaemonState::new_for_test().await;
    {
        let mut r = state.status.lock().unwrap();
        r.on_session_started(
            "s1",
            "host",
            "agent",
            "pk",
            ".",
            BTreeSet::from(["room".to_string()]),
            true,
            "T",
            "reading",
            100,
        )
        .unwrap();
        r.on_distill("s1", "T", "reviewing", 130).unwrap();
    }
    let fact = InputFact::StatusDrive(StatusDrive::DistillCompleted {
        session_id: "s1".into(),
        title: "T".into(),
        activity: "writing".into(),
        window_hash: Some("sha256:w2".into()),
        at: 160,
    });
    let v = acid_value(
        &state,
        &json!({ "verb": "acid", "handle": "status:s1", "fact": fact }),
    )
    .unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["necessary"], true);
    assert_eq!(v["unrelated_stable"], true);
}

#[tokio::test]
async fn subscription_acid_verifies_session_channel_cause() {
    let state = DaemonState::new_for_test().await;
    let mut sessions = BTreeMap::new();
    sessions.insert("s1".to_string(), BTreeSet::from(["room".to_string()]));
    let snapshot = CoverageSnapshot {
        daemon_channels: BTreeSet::new(),
        addressed_pubkeys: BTreeSet::new(),
        archived_channels: BTreeSet::new(),
        sessions: sessions.clone(),
    };
    state.subs.lock().unwrap().sync(&snapshot).unwrap();

    sessions.insert("s2".to_string(), BTreeSet::from(["room2".to_string()]));
    let fact = InputFact::SubscriptionSync {
        snapshot: CoverageSnapshot {
            daemon_channels: BTreeSet::new(),
            addressed_pubkeys: BTreeSet::new(),
            archived_channels: BTreeSet::new(),
            sessions,
        },
        at: 200,
    };
    let v = acid_value(
        &state,
        &json!({
            "verb": "acid",
            "handle": "sub:room",
            "cause": "subscriptions/session/s1/channels",
            "fact": fact,
        }),
    )
    .unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["necessary"], true);
    assert_eq!(v["unrelated_stable"], true);
}

#[tokio::test]
async fn turn_acid_verifies_turn_started_cause() {
    let state = DaemonState::new_for_test().await;
    {
        let mut r = state.turn_lifecycle.lock().unwrap();
        r.on_turn_started(
            crate::reconcile::TurnProjectionSeed {
                session_id: "s1".into(),
                working: false,
                turn_started_at: 0,
                transcript_ref: None,
            },
            100,
            None,
        )
        .unwrap();
        r.on_turn_started(
            crate::reconcile::TurnProjectionSeed {
                session_id: "s1".into(),
                working: true,
                turn_started_at: 100,
                transcript_ref: None,
            },
            130,
            None,
        )
        .unwrap();
    }
    let fact = InputFact::TurnStarted {
        session_id: "s1".into(),
        at: 160,
    };
    let v = acid_value(
        &state,
        &json!({ "verb": "acid", "handle": "turn:s1", "fact": fact }),
    )
    .unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["necessary"], true);
    assert_eq!(v["unrelated_stable"], true);
}

#[tokio::test]
async fn cursor_acid_verifies_observed_cursor_cause() {
    let state = DaemonState::new_for_test().await;
    {
        let mut r = state.cursor.lock().unwrap();
        r.request(
            crate::reconcile::CursorSeed {
                session_id: "s1".into(),
                seen_cursor: 10,
            },
            InputFact::TurnCheckRequested {
                session_id: "s1".into(),
                observed_cursor: 10,
                working: true,
                at: 20,
            },
        )
        .unwrap();
    }
    let fact = InputFact::TurnCheckRequested {
        session_id: "s1".into(),
        observed_cursor: 20,
        working: true,
        at: 30,
    };
    let v = acid_value(
        &state,
        &json!({
            "verb": "acid",
            "handle": "cursor:s1",
            "cause": "cursor/s1/observed_cursor",
            "fact": fact,
        }),
    )
    .unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["necessary"], true);
    assert_eq!(v["unrelated_stable"], true);
}

#[tokio::test]
async fn outbox_acid_verifies_relay_result_cause() {
    let state = DaemonState::new_for_test().await;
    {
        let mut r = state.outbox.lock().unwrap();
        r.drive(InputFact::OutboxEnqueueApplied {
            local_id: 7,
            event_id: "ev7".into(),
            event_hash: "sha256:event".into(),
            source_surface: "status".into(),
            source_ref: "status/s1#tx:1".into(),
            at: 100,
        })
        .unwrap();
        r.drive(InputFact::RelayPublishAccepted {
            local_id: 7,
            event_id: "ev7".into(),
            accepted: false,
            error: Some("relay rejected".into()),
            at: 110,
        })
        .unwrap();
    }
    let fact = InputFact::RelayPublishAccepted {
        local_id: 7,
        event_id: "ev7".into(),
        accepted: true,
        error: None,
        at: 120,
    };
    let v = acid_value(
        &state,
        &json!({
            "verb": "acid",
            "handle": "outbox:7",
            "cause": "outbox/7/result",
            "fact": fact,
        }),
    )
    .unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["necessary"], true);
    assert_eq!(v["unrelated_stable"], true);
}
