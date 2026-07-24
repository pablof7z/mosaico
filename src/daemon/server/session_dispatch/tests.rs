use super::*;
use crate::state::RegisterSession;
use nostr::ToBech32;

const WRITER_PUBKEY: &str = "31d4c4950a12b978cee21f84f4f5703e700b2d77a18648773239096675a7ab2d";

#[test]
fn fully_qualified_channels_use_leading_slash_paths() {
    assert_eq!(
        split_fully_qualified_channel("/workspace/research"),
        Some(("workspace", Some("research")))
    );
    assert_eq!(
        split_fully_qualified_channel("/workspace"),
        Some(("workspace", None))
    );
    assert_eq!(split_fully_qualified_channel("workspace.research"), None);
    assert_eq!(split_fully_qualified_channel("workspace/research"), None);
}

fn caller_session(state: &Arc<DaemonState>, channels: &[&str]) -> crate::state::Session {
    state.with_store(|s| {
        for channel in channels {
            s.upsert_channel(channel, channel, "", "", 1).unwrap();
        }
        s.reserve_hook_session_for_test(&RegisterSession {
            pubkey: "caller-pubkey".to_string(),
            observed_harness: "codex".to_string(),
            agent_slug: "codex".to_string(),
            channel_h: channels.first().copied().unwrap_or("project1").to_string(),
            child_pid: None,
            now: 1,
        })
        .unwrap();
        for (idx, channel) in channels.iter().enumerate().skip(1) {
            s.grant_session_route("caller-pubkey", channel, 2 + idx as u64)
                .unwrap();
        }
        s.get_session("caller-pubkey").unwrap().unwrap()
    })
}

#[tokio::test]
async fn route_channel_is_first_requested_channel_shared_with_caller() {
    let state = DaemonState::new_for_test().await;
    let caller = caller_session(&state, &["project1", "project1.bug-123"]);

    let route = first_shared_channel(
        &state,
        &caller,
        &["project2.qa".to_string(), "project1.bug-123".to_string()],
    )
    .unwrap();

    assert_eq!(route, "project1.bug-123");
}

#[tokio::test]
async fn route_channel_failure_lists_channels_the_caller_is_active_on() {
    let state = DaemonState::new_for_test().await;
    let caller = caller_session(&state, &["project1", "project1.dev"]);

    let err = first_shared_channel(&state, &caller, &["project2".to_string()])
        .unwrap_err()
        .to_string();

    assert!(err.contains("you need to specify a channel you're active on:"));
    assert!(err.contains("project1"));
    assert!(err.contains("project1.dev") || err.contains("@project1"));
}

#[test]
fn dispatch_message_body_prefixes_ack_pubkey_as_nostr_entity() {
    let body =
        dispatch_message_body("Hello! This is a quick connectivity test.", WRITER_PUBKEY).unwrap();
    let npub = PublicKey::from_hex(WRITER_PUBKEY)
        .unwrap()
        .to_bech32()
        .unwrap();

    assert_eq!(
        body,
        format!("nostr:{npub}: Hello! This is a quick connectivity test.")
    );
}

#[test]
fn dispatch_message_body_does_not_duplicate_existing_target_prefix() {
    let npub = PublicKey::from_hex(WRITER_PUBKEY)
        .unwrap()
        .to_bech32()
        .unwrap();
    let existing = format!("nostr:{npub}: already addressed");

    assert_eq!(
        dispatch_message_body(&existing, WRITER_PUBKEY).unwrap(),
        existing
    );
}

#[test]
fn dispatch_ack_observation_is_exactly_scoped_to_the_dispatch_event() {
    let query = dispatch_ack_query("dispatch-id");
    assert_eq!(
        query.kinds,
        std::collections::BTreeSet::from([crate::fabric::nip29::wire::KIND_STATUS])
    );
    assert_eq!(query.tag, Some(('e', "dispatch-id".to_string())));
}
