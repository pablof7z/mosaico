//! Hermetic explain-engine tests: a temp in-memory store, fixed timestamps, and
//! the real `changed_summary` join path (via `instrument::changed_summary_json`).

use super::*;
use crate::instrument::{changed_summary_json, window_hash};
use crate::state::llm_calls::NewLlmCall;
use crate::state::receipts::NewReceipt;

const SYS_MARKER: &str = "SYSTEM PROMPT MARKER";
const SLICE_MARKER: &str = "CURRENT TITLE: x\n\nTRANSCRIPT:\nuser: SLICE MARKER";

/// Record an llm_call + the status receipt of the 30315 it fed, joined by
/// `window_hash`, exactly as the runtime + status seam do at capture time.
fn seed_status_publish(store: &Store, session: &str, event_id: &str, created_at: i64) -> String {
    let wh = window_hash(SLICE_MARKER);
    store
        .record_llm_call(&NewLlmCall {
            pubkey: session.into(),
            window_hash: wh.clone(),
            provider: "claude-cli".into(),
            model: "claude-haiku".into(),
            system_prompt: SYS_MARKER.into(),
            transcript_slice: SLICE_MARKER.into(),
            raw_response: "TITLE: Fix bug\nNOW: reading logs".into(),
            parsed_title: Some("Fix bug".into()),
            parsed_activity: Some("reading logs".into()),
            created_at,
        })
        .unwrap();
    store
        .record_receipt(&NewReceipt {
            surface: "status".into(),
            transaction_id: 5,
            revision: 2,
            changed_summary: changed_summary_json(&[], &[], &[], Some(session), Some(&wh)),
            commands: r#"[{"kind":"replace","key":"status/sid-1","reason":"replace"}]"#.into(),
            artifact_ref: Some(event_id.into()),
            created_at,
        })
        .unwrap();
    wh
}

#[test]
fn explain_event_surfaces_the_llm_inputs() {
    let store = Store::open_memory().unwrap();
    seed_status_publish(&store, "sid-1", "evt-30315", 1_000);

    let v = explain(&store, &Handle::Event("evt-30315".into())).unwrap();

    // The receipt that explains this event was found by artifact_ref.
    assert_eq!(v["receipts"][0]["artifact_ref"], "evt-30315");
    assert_eq!(v["receipts"][0]["surface"], "status");
    // THE headline: the exact system prompt + transcript slice the LLM was fed.
    let llm = &v["llm_call"];
    assert_eq!(llm["system_prompt"], SYS_MARKER);
    assert!(llm["transcript_slice"]
        .as_str()
        .unwrap()
        .contains("SLICE MARKER"));
    assert_eq!(llm["model"], "claude-haiku");
    assert_eq!(llm["raw_response"], "TITLE: Fix bug\nNOW: reading logs");
    assert_eq!(llm["parsed_activity"], "reading logs");
}

#[test]
fn explain_event_without_llm_still_returns_receipt() {
    let store = Store::open_memory().unwrap();
    store
        .record_receipt(&NewReceipt {
            surface: "status".into(),
            transaction_id: 1,
            revision: 1,
            changed_summary: changed_summary_json(&[], &[], &[], Some("sid-2"), None),
            commands: "[]".into(),
            artifact_ref: Some("evt-nowh".into()),
            created_at: 10,
        })
        .unwrap();
    let v = explain(&store, &Handle::Event("evt-nowh".into())).unwrap();
    assert_eq!(v["receipts"][0]["artifact_ref"], "evt-nowh");
    assert!(v["llm_call"].is_null());
}

#[test]
fn explain_llm_reverse_joins_to_status_receipt() {
    let store = Store::open_memory().unwrap();
    seed_status_publish(&store, "sid-1", "evt-30315", 1_000);
    let v = explain(&store, &Handle::Llm(1)).unwrap();
    assert_eq!(v["llm_call"]["id"], 1);
    assert_eq!(v["receipts"][0]["artifact_ref"], "evt-30315");
}

#[test]
fn explain_session_at_ts_picks_nearest_llm_call() {
    let store = Store::open_memory().unwrap();
    for (wh_seed, ts) in [("a", 1_000_i64), ("b", 5_000), ("c", 9_000)] {
        store
            .record_llm_call(&NewLlmCall {
                pubkey: "sid-1".into(),
                window_hash: window_hash(wh_seed),
                provider: "p".into(),
                model: "m".into(),
                system_prompt: "s".into(),
                transcript_slice: wh_seed.into(),
                raw_response: "r".into(),
                parsed_title: None,
                parsed_activity: None,
                created_at: ts,
            })
            .unwrap();
    }
    let v = explain(
        &store,
        &Handle::Session {
            id: "sid-1".into(),
            at: Some(6_000),
        },
    )
    .unwrap();
    // 6_000 is closest to the 5_000 round-trip ("b").
    assert_eq!(v["llm_call"]["transcript_slice"], "b");
}

#[test]
fn explain_hook_filters_by_session_and_selects_nearest() {
    let store = Store::open_memory().unwrap();
    for (sess, kind, ts) in [
        ("sid-1", "turn_start", 100_i64),
        ("sid-1", "turn_check", 900),
    ] {
        store
            .record_receipt(&NewReceipt {
                surface: "hook_context".into(),
                transaction_id: 1,
                revision: 1,
                changed_summary: "{}".into(),
                commands: "[]".into(),
                artifact_ref: Some(format!("{sess}:{kind}:{ts}")),
                created_at: ts,
            })
            .unwrap();
    }
    // A different session's hook must not leak in.
    store
        .record_receipt(&NewReceipt {
            surface: "hook_context".into(),
            transaction_id: 1,
            revision: 1,
            changed_summary: "{}".into(),
            commands: "[]".into(),
            artifact_ref: Some("sid-2:turn_start:120".into()),
            created_at: 120,
        })
        .unwrap();
    for i in 0..600 {
        store
            .record_receipt(&NewReceipt {
                surface: "hook_context".into(),
                transaction_id: 1,
                revision: 1,
                changed_summary: "{}".into(),
                commands: "[]".into(),
                artifact_ref: Some(format!("sid-noise:turn_check:{}", 10_000 + i)),
                created_at: 10_000 + i,
            })
            .unwrap();
    }
    let v = explain(
        &store,
        &Handle::Hook {
            id: "sid-1".into(),
            at: Some(850),
        },
    )
    .unwrap();
    assert_eq!(v["receipts"].as_array().unwrap().len(), 1);
    assert_eq!(v["receipts"][0]["artifact_ref"], "sid-1:turn_check:900");
}

#[test]
fn parse_handle_covers_every_scheme() {
    assert_eq!(
        parse_handle("event:abcd").unwrap(),
        Handle::Event("abcd".into())
    );
    assert_eq!(parse_handle("llm:42").unwrap(), Handle::Llm(42));
    assert_eq!(
        parse_handle("session:sid-1@1234").unwrap(),
        Handle::Session {
            id: "sid-1".into(),
            at: Some(1234)
        }
    );
    assert_eq!(
        parse_handle("session:sid-1").unwrap(),
        Handle::Session {
            id: "sid-1".into(),
            at: None
        }
    );
    assert_eq!(
        parse_handle("hook:sid-1@9").unwrap(),
        Handle::Hook {
            id: "sid-1".into(),
            at: Some(9)
        }
    );
    assert_eq!(
        parse_handle("txn:status:7").unwrap(),
        Handle::Txn {
            surface: "status".into(),
            id: 7,
            at: None
        }
    );
    assert_eq!(
        parse_handle("txn:status:7@1234").unwrap(),
        Handle::Txn {
            surface: "status".into(),
            id: 7,
            at: Some(1234)
        }
    );
    assert_eq!(
        parse_handle("sub:proj-x").unwrap(),
        Handle::Sub {
            channel: "proj-x".into()
        }
    );
    assert!(parse_handle("bogus").is_err());
    assert!(parse_handle("mystery:1").is_err());
}
