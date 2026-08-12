use super::*;
use crate::state::RegisterSession;
use std::collections::{BTreeMap, BTreeSet};

const SCRIPTED_CLASSIFIED_FAILURE: &str =
    "fault=latched durability=absent reopen=required: Previous I/O error occurred";

fn ready_group_snapshot(
    group: &str,
    name: &str,
    admins: &[nostr::PublicKey],
    members: &[nostr::PublicKey],
) -> nmp::nip29::GroupSnapshot {
    use nmp::nip29::{GroupAvailability, GroupMetadata, HostRecords, ListedRecord, ListedSubject};
    let host = nostr::RelayUrl::parse("wss://relay.example").unwrap();
    let subjects = |keys: &[nostr::PublicKey]| {
        keys.iter()
            .map(|pubkey| ListedSubject {
                pubkey: *pubkey,
                role: None,
                hosts: BTreeSet::from([host.clone()]),
            })
            .collect::<Vec<_>>()
    };
    let admins = subjects(admins);
    let members = subjects(members);
    let record = |subjects| ListedRecord {
        subjects,
        as_of: nostr::Timestamp::from(2),
        event_id: nostr::EventId::all_zeros(),
        host: host.clone(),
    };
    let metadata = GroupMetadata {
        name: Some(name.to_string()),
        about: None,
        picture: None,
        tags: Vec::new(),
        as_of: nostr::Timestamp::from(1),
        event_id: nostr::EventId::all_zeros(),
        host: host.clone(),
    };
    nmp::nip29::GroupSnapshot {
        id: group.to_string(),
        metadata: Some(metadata.clone()),
        admins: admins.clone(),
        members: members.clone(),
        availability: GroupAvailability::Ready,
        per_host: BTreeMap::from([(
            host.clone(),
            HostRecords {
                metadata: Some(metadata),
                admins: Some(record(admins)),
                members: Some(record(members)),
                availability: GroupAvailability::Ready,
            },
        )]),
        disagreements: BTreeSet::new(),
    }
}

fn absent_group_snapshot(group: &str) -> nmp::nip29::GroupSnapshot {
    nmp::nip29::GroupSnapshot {
        id: group.to_string(),
        metadata: None,
        admins: Vec::new(),
        members: Vec::new(),
        availability: nmp::nip29::GroupAvailability::Ready,
        per_host: BTreeMap::new(),
        disagreements: BTreeSet::new(),
    }
}

fn register_caller(state: &Arc<DaemonState>, pubkey: &str) {
    state
        .with_store(|store| {
            store.install_test_nmp_group_delivery(crate::state::TestGroupDelivery::new([
                crate::state::TestGroup::new("project").metadata("project", "", "", 1),
            ]));
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
async fn channel_member_readiness_failure_reaches_actual_rpc_response() {
    let state = DaemonState::new_for_test_with_relays(vec![RELAY.into()]).await;
    let caller = Keys::generate().public_key().to_hex();
    let target = Keys::generate().public_key().to_hex();
    let management = state.backend_pubkey().unwrap();
    register_caller(&state, &caller);
    state
        .with_store(|store| {
            store.install_test_nmp_group_delivery(crate::state::TestGroupDelivery::new([
                crate::state::TestGroup::new("project")
                    .metadata("project", "", "", 1)
                    .admins(vec![management.clone()])
                    .members(Vec::new()),
            ]));
            Ok::<_, anyhow::Error>(())
        })
        .unwrap();
    let project = ready_group_snapshot(
        "project",
        "project",
        &[nostr::PublicKey::from_hex(&management).unwrap()],
        &[],
    );
    for _ in 0..5 {
        state.nmp().script_group_snapshot(project.clone());
    }
    state
        .nmp()
        .script_write_error("scripted NMP publish refusal", SCRIPTED_CLASSIFIED_FAILURE);

    let response = super::super::super::dispatch(
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
    assert!(error.message.contains(SCRIPTED_CLASSIFIED_FAILURE));
    assert!(error
        .message
        .contains("9000 put-user (session) NMP publish failed"));
    assert!(!error.message.contains("member add for"));
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
            store.install_test_nmp_group_delivery(crate::state::TestGroupDelivery::new([
                crate::state::TestGroup::new("project")
                    .metadata("project", "", "", 1)
                    .admins(vec![management.clone()])
                    .members(Vec::new()),
            ]));
            store.reserve_channel_resolution_intent("project", "new-channel", "child-h", 4)?;
            Ok::<_, anyhow::Error>(())
        })
        .unwrap();
    let absent_child = absent_group_snapshot("child-h");
    let project = ready_group_snapshot(
        "project",
        "project",
        &[nostr::PublicKey::from_hex(&management).unwrap()],
        &[],
    );
    state.nmp().script_group_snapshot(absent_child.clone());
    for _ in 0..6 {
        state.nmp().script_group_snapshot(project.clone());
    }
    state.nmp().script_group_snapshot(absent_child);
    state
        .nmp()
        .script_write_error("scripted NMP publish refusal", SCRIPTED_CLASSIFIED_FAILURE);

    let response = super::super::super::dispatch(
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
    assert!(error.message.contains(SCRIPTED_CLASSIFIED_FAILURE));
    assert!(error
        .message
        .contains("9007 create-subgroup NMP publish failed"));
    assert!(!error.message.contains("does the relay support"));
    eprintln!(
        "CORPUS_CHANNEL_CREATE_RPC={}",
        serde_json::to_string(&error).unwrap()
    );
}
