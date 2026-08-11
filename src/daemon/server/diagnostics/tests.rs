use super::super::*;
use crate::daemon::protocol::Request;
use crate::state::RegisterSession;
use nostr::{Event, EventBuilder, Keys, Kind, Tag};

#[path = "tests/status.rs"]
mod status;

const RELAY: &str = "wss://relay.example.com";
// Scripted future-classified receipt matching newer NMP behavior. The pinned
// NMP revision cannot originate this classification itself.
const SCRIPTED_CLASSIFIED_FAILURE: &str =
    "fault=latched durability=absent reopen=required: Previous I/O error occurred";

fn event(kind: u16, tags: Vec<Tag>) -> Event {
    EventBuilder::new(Kind::from(kind), "")
        .tags(tags)
        .sign_with_keys(&Keys::generate())
        .unwrap()
}

/// The relay-signed records a host holds for `group`. Readiness still asks the
/// WIRE whether the group exists — a cached `relay_channels` row may be the
/// local reservation `channel_init` writes before provisioning — so this stays
/// scripted. Only the ROSTER moved to the cache.
fn group_records(group: &str, management: &str) -> Vec<Event> {
    use crate::fabric::nip29::wire::{KIND_GROUP_ADMINS, KIND_GROUP_MEMBERS, KIND_GROUP_METADATA};
    vec![
        event(
            KIND_GROUP_METADATA,
            vec![
                Tag::parse(["d", group]).unwrap(),
                Tag::parse(["name", "project"]).unwrap(),
            ],
        ),
        event(
            KIND_GROUP_ADMINS,
            vec![
                Tag::parse(["d", group]).unwrap(),
                Tag::parse(["p", management, "admin"]).unwrap(),
            ],
        ),
        event(KIND_GROUP_MEMBERS, vec![Tag::parse(["d", group]).unwrap()]),
    ]
}

fn register_caller(state: &Arc<DaemonState>, pubkey: &str) {
    state
        .with_store(|store| {
            store.upsert_channel("project", "project", "", "", 1)?;
            store.reserve_hook_session_for_test(&RegisterSession {
                pubkey: pubkey.into(),
                observed_harness: "codex".into(),
                agent_slug: "caller".into(),
                launch_channel_h: "project".into(),
                work_root: "project".into(),
                child_pid: None,
                now: 1,
            })
        })
        .unwrap();
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
        .nmp()
        .publish_group("project", EventBuilder::new(Kind::TextNote, "owed"), &keys)
        .expect("acceptance never depends on a relay");
    state.nmp().script_read_settled_events(Vec::new());

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
    state.nmp().script_read_timed_out_events(vec![event(
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
    state.nmp().script_disconnected_read();

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

#[tokio::test]
async fn channel_member_readiness_failure_reaches_actual_rpc_response() {
    let state = DaemonState::new_for_test_with_relays(vec![RELAY.into()]).await;
    let caller = Keys::generate().public_key().to_hex();
    let target = Keys::generate().public_key().to_hex();
    let management = state.backend_pubkey().unwrap();
    register_caller(&state, &caller);

    // Existence is still proven from the wire; the ROSTER now comes from the
    // cache the retained group-records observation keeps current.
    state
        .with_store(|store| {
            store.replace_channel_admins("project", std::slice::from_ref(&management), 2)?;
            store.replace_channel_members("project", &[], 3)
        })
        .unwrap();
    state
        .nmp()
        .script_read_settled_events(group_records("project", &management));
    state
        .nmp()
        .script_write_error("scripted NMP publish refusal", SCRIPTED_CLASSIFIED_FAILURE);
    state.nmp().script_read_settled_events(Vec::new());

    let response = super::super::dispatch(
        &state,
        &Request {
            id: 702,
            method: "channel_add_member".into(),
            params: serde_json::json!({
                "channel": "#project",
                "pubkey": target,
                "session": caller,
                "admin": false
            }),
        },
    )
    .await;
    let error = response.error.expect("actual RPC failure response");
    assert!(
        error.message.contains(SCRIPTED_CLASSIFIED_FAILURE),
        "{}",
        error.message
    );
    assert!(
        error
            .message
            .contains("9000 put-user (session) NMP publish failed"),
        "{}",
        error.message
    );
    assert!(
        !error.message.contains("member add for"),
        "generic replacement wording escaped: {}",
        error.message
    );
    eprintln!(
        "CORPUS_CHANNEL_MEMBER_RPC={}",
        serde_json::to_string(&error).unwrap()
    );
}

#[tokio::test]
async fn channel_create_readiness_failure_reaches_actual_rpc_response() {
    let state = DaemonState::new_for_test_with_relays(vec![RELAY.into()]).await;
    let management = state.backend_pubkey().unwrap();
    state
        .with_store(|store| {
            store.upsert_channel("project", "project", "", "", 1)?;
            store.replace_channel_admins("project", std::slice::from_ref(&management), 2)?;
            store.replace_channel_members("project", &[], 3)
        })
        .unwrap();
    state.nmp().script_read_settled_events(Vec::new());
    state
        .nmp()
        .script_write_error("scripted NMP publish refusal", SCRIPTED_CLASSIFIED_FAILURE);
    state.nmp().script_read_settled_events(Vec::new());

    let response = super::super::dispatch(
        &state,
        &Request {
            id: 703,
            method: "channel_create".into(),
            params: serde_json::json!({
                "channel": "#project/new-channel",
                "about": "",
                "agents": []
            }),
        },
    )
    .await;
    let error = response.error.expect("actual channel_create RPC failure");
    assert!(
        error.message.contains(SCRIPTED_CLASSIFIED_FAILURE),
        "{}",
        error.message
    );
    assert!(
        error
            .message
            .contains("9007 create-subgroup NMP publish failed"),
        "{}",
        error.message
    );
    assert!(
        !error.message.contains("does the relay support"),
        "generic replacement wording escaped: {}",
        error.message
    );
    eprintln!(
        "CORPUS_CHANNEL_CREATE_RPC={}",
        serde_json::to_string(&error).unwrap()
    );
}
