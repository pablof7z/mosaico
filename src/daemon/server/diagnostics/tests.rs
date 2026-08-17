use super::super::*;
use crate::daemon::protocol::Request;
use nostr::{Event, EventBuilder, Keys, Kind, Tag};

#[path = "tests/readiness.rs"]
mod readiness;
#[path = "tests/status.rs"]
mod status;

const RELAY: &str = "wss://relay.example.com";

fn event(kind: u16, tags: Vec<Tag>) -> Event {
    EventBuilder::new(Kind::from(kind), "")
        .tags(tags)
        .sign_with_keys(&Keys::generate())
        .unwrap()
}

/// NMP's durable publish queue is the doctor's ONE account of outstanding
/// writes. It replaced a process-local receipt observer that reported the same
/// question from a second, non-durable source: a daemon restarted with parked
/// writes used to get a clean bill from the observer it had no basis for.
#[tokio::test]
async fn doctor_rpc_reports_the_durable_publish_queue() {
    let state = DaemonState::new_for_test_with_relays(vec![RELAY.into()]).await;
    let keys = Keys::generate();
    state
        .snapshot()
        .nmp
        .publish_group("project", EventBuilder::new(Kind::TextNote, "owed"), &keys)
        .expect("acceptance never depends on a relay");
    state.snapshot().nmp.script_read_settled_events(Vec::new());

    let response = super::super::dispatch(
        &state,
        &Request {
            id: 706,
            method: "doctor".into(),
            params: serde_json::json!({}),
        },
    )
    .await;
    let json = response.ok.expect("doctor RPC response");
    assert_eq!(json["write_probe"]["publish"]["status"], "skipped");
    assert_eq!(json["write_probe"]["readback"]["status"], "verified");
    assert_eq!(
        json["write_probe"]["readback"]["acquisition"]["termination"],
        "relay_settled"
    );
    let queue = &json["publish_queue"];
    assert!(queue.is_object(), "{json}");
    assert!(queue["entries"].is_u64(), "{queue}");
    assert!(queue["outstanding"].is_u64(), "{queue}");
    assert!(queue["stuck"].is_array(), "{queue}");
    // An empty queue must never be spelled the same way as an unreadable one.
    assert!(queue.get("unreadable").is_none(), "{queue}");
    // The write just accepted is what the daemon still owes, and the queue is
    // the only place that is now recorded.
    assert_eq!(queue["outstanding"], 1, "{queue}");
    // Nothing about it needs a person: a signer is attached and the route is
    // explicit, so it is in flight rather than stuck.
    assert!(queue["stuck"].as_array().unwrap().is_empty(), "{queue}");
}

#[tokio::test]
async fn doctor_rpc_never_reports_cached_rows_as_current_relay_io() {
    let state = DaemonState::new_for_test_with_relays(vec![RELAY.into()]).await;
    state
        .snapshot()
        .nmp
        .script_read_timed_out_events(vec![event(
            crate::fabric::nip29::wire::KIND_GROUP_METADATA,
            vec![Tag::parse(["d", "cached-only"]).unwrap()],
        )]);

    let response = super::super::dispatch(
        &state,
        &Request {
            id: 707,
            method: "doctor".into(),
            params: serde_json::json!({}),
        },
    )
    .await;
    let json = response.ok.expect("doctor RPC response");
    let readback = &json["write_probe"]["readback"];

    assert_eq!(json["write_probe"]["publish"]["status"], "skipped");
    assert_eq!(readback["status"], "failed");
    assert_eq!(readback["acquisition"]["termination"], "timed_out");
    assert_eq!(
        readback["acquisition"]["branches"][0]["sources"][0]["status"],
        "Requesting"
    );
    assert!(readback["summary"]
        .as_str()
        .unwrap()
        .contains("1 cached/current event"));
}

#[tokio::test]
async fn doctor_rpc_reports_a_disconnected_source_after_the_engine_started() {
    let state = DaemonState::new_for_test_with_relays(vec![RELAY.into()]).await;
    state.snapshot().nmp.script_disconnected_read();

    let response = super::super::dispatch(
        &state,
        &Request {
            id: 708,
            method: "doctor".into(),
            params: serde_json::json!({}),
        },
    )
    .await;
    let json = response.ok.expect("doctor RPC response");
    let readback = &json["write_probe"]["readback"];

    assert_eq!(readback["status"], "failed");
    assert_eq!(
        readback["acquisition"]["termination"],
        "subscription_closed"
    );
    assert_eq!(
        readback["acquisition"]["branches"][0]["sources"][0]["status"],
        "Disconnected"
    );
}
