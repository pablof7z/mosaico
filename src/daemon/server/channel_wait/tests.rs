use super::*;

#[path = "tests/active_registry.rs"]
mod active_registry;
#[path = "tests/ambient.rs"]
mod ambient;
use crate::state::{
    Message, Profile, RegisterSession, RelayEvent, TestGroup, TestGroupDelivery, TestRelayDelivery,
};

const SELF_PUBKEY: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const X_CHANNEL: &str = "x-h";
const Y_CHANNEL: &str = "y-h";
const Z_CHANNEL: &str = "z-h";

fn seed_session(state: &Arc<DaemonState>) -> Session {
    state
        .with_store(|store| {
            store.install_test_nmp_group_delivery(TestGroupDelivery::new([
                TestGroup::new("root").metadata("root", "", "", 1),
                TestGroup::new(X_CHANNEL).metadata("x", "", "root", 2),
                TestGroup::new(Y_CHANNEL).metadata("y", "", "root", 3),
                TestGroup::new(Z_CHANNEL).metadata("z", "", "root", 4),
            ]));
            store.reserve_hook_session_for_test(&RegisterSession {
                pubkey: SELF_PUBKEY.into(),
                observed_harness: "codex".into(),
                agent_slug: "self".into(),
                launch_channel_h: X_CHANNEL.into(),
                work_root: "root".into(),
                child_pid: None,
                now: 1,
            })?;
            store.grant_session_route(SELF_PUBKEY, X_CHANNEL, 1)?;
            store.grant_session_route(SELF_PUBKEY, Y_CHANNEL, 2)?;
            store.get_session(SELF_PUBKEY)?.context("missing session")
        })
        .unwrap()
}

fn deliver_chat(
    state: &Arc<DaemonState>,
    id: &str,
    channel: &str,
    author: &str,
    body: &str,
    reply_to: Option<&str>,
) {
    let tags = reply_to
        .map(|target| serde_json::json!([["e", target]]).to_string())
        .unwrap_or_else(|| "[]".to_string());
    state
        .with_store(|store| {
            let mut events = store.events_by_kind(9, u32::MAX)?;
            events.push(RelayEvent {
                id: id.into(),
                kind: 9,
                pubkey: author.into(),
                created_at: 10,
                channel_h: channel.into(),
                d_tag: String::new(),
                content: body.into(),
                tags_json: tags,
            });
            store.install_test_nmp_relay_delivery(TestRelayDelivery::new().events(events));
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();
}

#[tokio::test]
async fn no_channel_uses_all_joined_channels_and_explicit_channels_narrow() {
    let state = DaemonState::new_for_test().await;
    let rec = seed_session(&state);

    assert_eq!(
        resolve_joined_scopes(&state, &rec, &[]).unwrap(),
        [X_CHANNEL, Y_CHANNEL]
    );
    assert_eq!(
        resolve_joined_scopes(&state, &rec, &["#root/y".into()]).unwrap(),
        [Y_CHANNEL]
    );
    let error = resolve_joined_scopes(&state, &rec, &["#root/z".into()]).unwrap_err();
    assert!(error.to_string().contains("has not joined channel"));
    // A bare opaque id is not an accepted reference form.
    let bare = resolve_joined_scopes(&state, &rec, &[Y_CHANNEL.into()]).unwrap_err();
    assert!(bare.to_string().contains("has not joined channel"));
}

#[tokio::test]
async fn explicit_channel_filters_resolve_across_every_joined_workspace() {
    let state = DaemonState::new_for_test().await;
    let rec = seed_session(&state);
    state
        .with_store(|store| {
            store.install_test_nmp_group_delivery(TestGroupDelivery::new([
                TestGroup::new("root").metadata("root", "", "", 1),
                TestGroup::new(X_CHANNEL).metadata("x", "", "root", 2),
                TestGroup::new(Y_CHANNEL).metadata("y", "", "root", 3),
                TestGroup::new(Z_CHANNEL).metadata("z", "", "root", 4),
                TestGroup::new("other").metadata("other", "", "", 5),
                TestGroup::new("other-y").metadata("y", "", "other", 6),
            ]));
            store.grant_session_route(SELF_PUBKEY, "other-y", 7)?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    assert_eq!(
        resolve_joined_scopes(&state, &rec, &["#other/y".into()]).unwrap(),
        ["other-y"]
    );
    assert_eq!(
        resolve_joined_scopes(&state, &rec, &["#root/y".into()]).unwrap(),
        [Y_CHANNEL]
    );
    // A bare opaque id is rejected even when two joined channels share a
    // human-facing leaf name.
    let bare = resolve_joined_scopes(&state, &rec, &[Y_CHANNEL.into()]).unwrap_err();
    assert!(bare.to_string().contains("has not joined channel"));
}

#[tokio::test]
async fn correlated_wait_skips_unrelated_chat_and_returns_exact_reply() {
    let state = DaemonState::new_for_test().await;
    let rec = seed_session(&state);
    deliver_chat(
        &state,
        "original",
        X_CHANNEL,
        SELF_PUBKEY,
        "please reply",
        None,
    );
    let mut cursor = state
        .with_store(|store| store.message_arrival_sequence("original"))
        .unwrap()
        .unwrap();
    deliver_chat(&state, "noise", X_CHANNEL, "noise-pk", "noise", None);
    deliver_chat(
        &state,
        "reply",
        X_CHANNEL,
        "peer-pk",
        "done",
        Some("original"),
    );
    let filter =
        AuthorFilter::from_params(&state, &[X_CHANNEL.into()], &WaitParams::default()).unwrap();

    let found = drain_matching(
        &state,
        &mut cursor,
        &[X_CHANNEL.into()],
        Some("original"),
        &filter,
        &own_pubkeys(&rec),
        &state.backend_pubkey().unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(found.message_id, "reply");
}

#[tokio::test]
async fn ambient_wait_excludes_management_and_callers_own_chat() {
    let state = DaemonState::new_for_test().await;
    let rec = seed_session(&state);
    let mut cursor = state
        .with_store(|store| store.latest_message_arrival_sequence())
        .unwrap();
    deliver_chat(&state, "self-chat", X_CHANNEL, SELF_PUBKEY, "mine", None);
    deliver_chat(
        &state,
        "management-chat",
        X_CHANNEL,
        &state.backend_pubkey().unwrap(),
        "mgmt ok",
        None,
    );
    deliver_chat(&state, "human-chat", X_CHANNEL, "human-pk", "hello", None);
    let filter =
        AuthorFilter::from_params(&state, &[X_CHANNEL.into()], &WaitParams::default()).unwrap();

    let found = drain_matching(
        &state,
        &mut cursor,
        &[X_CHANNEL.into()],
        None,
        &filter,
        &own_pubkeys(&rec),
        &state.backend_pubkey().unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(found.message_id, "human-chat");
}

#[tokio::test]
async fn from_filter_resolves_a_human_member_across_the_channel_union() {
    let state = DaemonState::new_for_test().await;
    seed_session(&state);
    state
        .with_store(|store| {
            store.install_test_nmp_relay_delivery(TestRelayDelivery::new().profiles([Profile {
                pubkey: "human-pk".into(),
                name: "pablo".into(),
                slug: "pablo".into(),
                agent_slug: String::new(),
                host: String::new(),
                is_backend: false,
                agents: Vec::new(),
                workspaces: Vec::new(),
                updated_at: 1,
            }]));
            store.install_test_nmp_group_delivery(TestGroupDelivery::new([
                TestGroup::new("root").metadata("root", "", "", 1),
                TestGroup::new(X_CHANNEL).metadata("x", "", "root", 2),
                TestGroup::new(Y_CHANNEL)
                    .metadata("y", "", "root", 3)
                    .members(vec!["human-pk".into()]),
                TestGroup::new(Z_CHANNEL).metadata("z", "", "root", 4),
            ]));
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();
    let params = WaitParams {
        from: Some("pablo".into()),
        ..WaitParams::default()
    };
    let filter =
        AuthorFilter::from_params(&state, &[X_CHANNEL.into(), Y_CHANNEL.into()], &params).unwrap();
    let message = Message {
        message_id: "human-message".into(),
        channel_h: Y_CHANNEL.into(),
        author_pubkey: "human-pk".into(),
        body: "hello".into(),
        created_at: 1,
        attachment_dir: String::new(),
    };

    assert!(filter.matches(&state, &message));
}
