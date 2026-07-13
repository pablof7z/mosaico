use super::*;
use crate::state::{RecordMessage, RegisterSession, RelayEvent};

fn seed_session(state: &Arc<DaemonState>) -> Session {
    state
        .with_store(|store| {
            store.upsert_channel("root", "root", "", "", 1)?;
            store.upsert_channel("x", "x", "", "root", 2)?;
            store.upsert_channel("y", "y", "", "root", 3)?;
            store.upsert_channel("z", "z", "", "root", 4)?;
            store.upsert_session_row(
                "self-session",
                &RegisterSession {
                    harness: "codex".into(),
                    external_id_kind: "harness_session".into(),
                    external_id: "self-session".into(),
                    agent_pubkey: "self-pubkey".into(),
                    agent_slug: "self".into(),
                    channel_h: "x".into(),
                    child_pid: None,
                    transcript_path: None,
                    resume_id: String::new(),
                    now: 1,
                },
            )?;
            store.join_session_channel("self-session", "x", 1)?;
            store.join_session_channel("self-session", "y", 2)?;
            store
                .get_session("self-session")?
                .context("missing session")
        })
        .unwrap()
}

fn insert_chat(
    state: &Arc<DaemonState>,
    id: &str,
    channel: &str,
    author: &str,
    author_session: Option<&str>,
    body: &str,
    reply_to: Option<&str>,
) {
    let tags = reply_to
        .map(|target| serde_json::json!([["e", target]]).to_string())
        .unwrap_or_else(|| "[]".to_string());
    state
        .with_store(|store| {
            store.insert_event(&RelayEvent {
                id: id.into(),
                kind: 9,
                pubkey: author.into(),
                created_at: 10,
                channel_h: channel.into(),
                d_tag: String::new(),
                content: body.into(),
                tags_json: tags,
            })?;
            store.record_message(&RecordMessage {
                message_id: id.into(),
                thread_id: channel.into(),
                channel_h: channel.into(),
                author_pubkey: author.into(),
                author_session: author_session.map(str::to_string),
                body: body.into(),
                created_at: 10,
                direction: "inbound".into(),
                sync_state: "accepted".into(),
                native_event_id: Some(id.into()),
                error: None,
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();
}

#[tokio::test]
async fn no_channel_uses_all_active_channels_and_explicit_channels_narrow() {
    let state = DaemonState::new_for_test().await;
    let rec = seed_session(&state);

    assert_eq!(
        resolve_active_scopes(&state, &rec, &[]).unwrap(),
        ["x", "y"]
    );
    assert_eq!(
        resolve_active_scopes(&state, &rec, &["y".into()]).unwrap(),
        ["y"]
    );
    let error = resolve_active_scopes(&state, &rec, &["z".into()]).unwrap_err();
    assert!(error.to_string().contains("not active on channel"));
}

#[tokio::test]
async fn explicit_channel_filters_resolve_across_every_active_workspace() {
    let state = DaemonState::new_for_test().await;
    let rec = seed_session(&state);
    state
        .with_store(|store| {
            store.upsert_channel("other", "other", "", "", 5)?;
            store.upsert_channel("other-y", "y", "", "other", 6)?;
            store.join_session_channel("self-session", "other-y", 7)?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    assert_eq!(
        resolve_active_scopes(&state, &rec, &["other.y".into()]).unwrap(),
        ["other-y"]
    );
    let error = resolve_active_scopes(&state, &rec, &["y".into()]).unwrap_err();
    assert!(error
        .to_string()
        .contains("ambiguous among active channels"));
}

#[tokio::test]
async fn correlated_wait_skips_unrelated_chat_and_returns_exact_reply() {
    let state = DaemonState::new_for_test().await;
    let rec = seed_session(&state);
    insert_chat(
        &state,
        "original",
        "x",
        "self-pubkey",
        Some("self-session"),
        "please reply",
        None,
    );
    let mut cursor = state
        .with_store(|store| store.message_rowid("original"))
        .unwrap()
        .unwrap();
    insert_chat(&state, "noise", "x", "noise-pk", None, "noise", None);
    insert_chat(
        &state,
        "reply",
        "x",
        "peer-pk",
        None,
        "done",
        Some("original"),
    );
    let filter = AuthorFilter::from_params(&state, &["x".into()], &WaitParams::default()).unwrap();

    let found = drain_matching(
        &state,
        &mut cursor,
        &["x".into()],
        Some("original"),
        &filter,
        &own_pubkeys(&state, &rec),
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
        .with_store(|store| store.latest_message_rowid())
        .unwrap();
    insert_chat(
        &state,
        "self-chat",
        "x",
        "self-pubkey",
        Some("self-session"),
        "mine",
        None,
    );
    insert_chat(
        &state,
        "management-chat",
        "x",
        &state.backend_pubkey().unwrap(),
        None,
        "mgmt ok",
        None,
    );
    insert_chat(&state, "human-chat", "x", "human-pk", None, "hello", None);
    let filter = AuthorFilter::from_params(&state, &["x".into()], &WaitParams::default()).unwrap();

    let found = drain_matching(
        &state,
        &mut cursor,
        &["x".into()],
        None,
        &filter,
        &own_pubkeys(&state, &rec),
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
            store.upsert_profile("human-pk", "pablo", "pablo", "", false, 1)?;
            store.upsert_channel_member("y", "human-pk", "member", 1)?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();
    let params = WaitParams {
        from: Some("pablo".into()),
        ..WaitParams::default()
    };
    let filter = AuthorFilter::from_params(&state, &["x".into(), "y".into()], &params).unwrap();
    let message = Message {
        message_id: "human-message".into(),
        thread_id: "y".into(),
        channel_h: "y".into(),
        author_pubkey: "human-pk".into(),
        author_session: None,
        body: "hello".into(),
        created_at: 1,
        direction: "inbound".into(),
        sync_state: "accepted".into(),
        native_event_id: Some("human-message".into()),
        error: None,
    };

    assert!(filter.matches(&state, &message));
}

#[tokio::test]
async fn ambient_rpc_returns_first_new_chat_from_any_active_channel() {
    let state = DaemonState::new_for_test().await;
    seed_session(&state);
    let waiting = {
        let state = state.clone();
        tokio::spawn(async move {
            rpc_channel_wait(
                &state,
                &serde_json::json!({
                    "session": "self-session",
                    "timeout_secs": 2,
                }),
            )
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(30)).await;
    insert_chat(&state, "new-chat", "y", "peer-pk", None, "hello", None);
    state.emit_tail(TailEvent::Msg {
        ts: 10,
        channel: "y".into(),
        from: "peer".into(),
        from_session: None,
        to: "channel-chat".into(),
        to_session: None,
        body: "hello".into(),
    });

    let result = waiting.await.unwrap().unwrap();
    assert_eq!(result["outcome"], "message");
    assert_eq!(result["message"]["event_id"], "new-chat");
    assert_eq!(result["message"]["channel_ref"], "root.y");
}

#[tokio::test]
async fn timeout_is_a_normal_structured_outcome() {
    let state = DaemonState::new_for_test().await;
    seed_session(&state);
    let result = rpc_channel_wait(
        &state,
        &serde_json::json!({
            "session": "self-session",
            "timeout_secs": 1,
            "channels": ["x"],
        }),
    )
    .await
    .unwrap();

    assert_eq!(result["outcome"], "timeout");
    assert_eq!(result["timeout_secs"], 1);
    assert_eq!(result["channels"], serde_json::json!(["root.x"]));
}
