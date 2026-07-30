use super::*;
use crate::state::receipts::NewReceipt;

fn status_summary(pubkey: &str) -> String {
    serde_json::json!({ "pubkey": pubkey }).to_string()
}

#[test]
fn event_resolves_matching_receipts() {
    let store = Store::open_memory().unwrap();
    store
        .record_receipt(&NewReceipt {
            surface: "status".into(),
            transaction_id: 5,
            revision: 2,
            changed_summary: status_summary("sid-1"),
            commands: "[]".into(),
            artifact_ref: Some("event-1".into()),
            created_at: 10,
        })
        .unwrap();

    let value = explain(&store, &Handle::Event("event-1".into())).unwrap();
    assert_eq!(value["kind"], "event");
    assert_eq!(value["receipts"][0]["artifact_ref"], "event-1");
}

#[test]
fn session_resolves_latest_matching_status_receipt() {
    let store = Store::open_memory().unwrap();
    for (pubkey, at) in [("sid-1", 10), ("sid-2", 20), ("sid-1", 30)] {
        store
            .record_receipt(&NewReceipt {
                surface: "status".into(),
                transaction_id: at,
                revision: 1,
                changed_summary: status_summary(pubkey),
                commands: "[]".into(),
                artifact_ref: Some(format!("event-{at}")),
                created_at: at,
            })
            .unwrap();
    }

    let value = explain(
        &store,
        &Handle::Session {
            id: "sid-1".into(),
            at: None,
        },
    )
    .unwrap();
    assert_eq!(value["receipts"][0]["artifact_ref"], "event-30");
}

#[test]
fn hook_filters_by_session_and_selects_nearest() {
    let store = Store::open_memory().unwrap();
    for (session, kind, at) in [
        ("sid-1", "turn_start", 100_i64),
        ("sid-1", "turn_check", 900),
        ("sid-2", "turn_start", 120),
    ] {
        store
            .record_receipt(&NewReceipt {
                surface: "hook_context".into(),
                transaction_id: 1,
                revision: 1,
                changed_summary: "{}".into(),
                commands: "[]".into(),
                artifact_ref: Some(format!("{session}:{kind}:{at}")),
                created_at: at,
            })
            .unwrap();
    }

    let value = explain(
        &store,
        &Handle::Hook {
            id: "sid-1".into(),
            at: Some(850),
        },
    )
    .unwrap();
    assert_eq!(value["receipts"].as_array().unwrap().len(), 1);
    assert_eq!(value["receipts"][0]["artifact_ref"], "sid-1:turn_check:900");
}

#[test]
fn parse_handle_exposes_only_current_schemes() {
    assert_eq!(
        parse_handle("event:abcd").unwrap(),
        Handle::Event("abcd".into())
    );
    assert_eq!(
        parse_handle("session:sid-1@1234").unwrap(),
        Handle::Session {
            id: "sid-1".into(),
            at: Some(1234),
        }
    );
    assert_eq!(
        parse_handle("hook:sid-1@9").unwrap(),
        Handle::Hook {
            id: "sid-1".into(),
            at: Some(9),
        }
    );

    assert!(parse_handle("bogus").is_err());
    assert!(parse_handle("mystery:1").is_err());
}
