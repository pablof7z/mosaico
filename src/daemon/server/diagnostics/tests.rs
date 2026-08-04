use super::super::*;
use crate::daemon::protocol::Request;
use crate::state::RegisterSession;
use nmp::WriteStatus;
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

#[tokio::test]
async fn profile_receipt_reaches_actual_doctor_rpc_json() {
    let state = DaemonState::new_for_test_with_relays(vec![RELAY.into()]).await;
    let profile = EventBuilder::new(Kind::Metadata, "{}")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    state.nmp.script_write_statuses(vec![
        WriteStatus::Accepted,
        WriteStatus::Signed(profile.id),
        WriteStatus::Failed(SCRIPTED_CLASSIFIED_FAILURE.into()),
    ]);
    state.nmp.enqueue_profile_event(&profile).unwrap();
    state.nmp.wait_background_receipts();
    state.nmp.script_read_events(Vec::new());

    let response = super::super::dispatch(
        &state,
        &Request {
            id: 701,
            method: "doctor".into(),
            params: serde_json::json!({}),
        },
    )
    .await;
    let json = response.ok.expect("doctor RPC response");
    let failure = &json["background_writes"]["last_failure"];
    assert_eq!(failure["status"], "failed");
    assert_eq!(failure["operation"], "profile");
    assert_eq!(failure["source_ref"], profile.id.to_hex());
    assert_eq!(failure["detail"], SCRIPTED_CLASSIFIED_FAILURE);
    eprintln!(
        "CORPUS_DOCTOR_JSON={}",
        serde_json::to_string(&json).unwrap()
    );
}

#[tokio::test]
async fn partial_submission_cause_remains_retrievable_from_doctor_rpc() {
    let state = DaemonState::new_for_test_with_relays(vec![
        "wss://relay-a.example.com".into(),
        "wss://relay-b.example.com".into(),
    ])
    .await;
    let profile = EventBuilder::new(Kind::Metadata, "{}")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    state
        .nmp
        .script_write_statuses(vec![WriteStatus::Failed("prior receipt retained".into())]);
    state.nmp.script_write_error(
        "intent 1 durable-store persistence failure [fault=latched durability=absent reopen=required]",
        "Previous I/O error occurred",
    );
    let enqueue = state.nmp.enqueue_profile_event(&profile).unwrap_err();
    assert!(format!("{enqueue:#}").contains("Previous I/O error occurred"));
    state.nmp.wait_background_receipts();
    state.nmp.script_read_events(Vec::new());

    let response = super::super::dispatch(
        &state,
        &Request {
            id: 704,
            method: "doctor".into(),
            params: serde_json::json!({}),
        },
    )
    .await;
    let json = response.ok.expect("doctor RPC response");
    let writes = &json["background_writes"];
    assert_eq!(writes["last_failure"]["status"], "submission_failed");
    assert!(writes["last_failure"]["detail"]
        .as_str()
        .unwrap()
        .contains("Previous I/O error occurred"));
    let history = writes["recent_failures"].as_array().unwrap();
    assert!(history.iter().any(|evidence| {
        evidence["status"] == "failed" && evidence["detail"] == "prior receipt retained"
    }));
    eprintln!(
        "CORPUS_PARTIAL_DOCTOR_JSON={}",
        serde_json::to_string(&json).unwrap()
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
        .nmp
        .script_read_events(group_records("project", &management));
    state.nmp.script_write_statuses(vec![WriteStatus::Failed(
        SCRIPTED_CLASSIFIED_FAILURE.into(),
    )]);
    state.nmp.script_read_events(Vec::new());

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
    state.nmp.script_read_events(Vec::new());
    state.nmp.script_write_statuses(vec![WriteStatus::Failed(
        SCRIPTED_CLASSIFIED_FAILURE.into(),
    )]);
    state.nmp.script_read_events(Vec::new());

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
