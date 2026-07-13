use super::channel_membership_rpc::resolve_caller;
use super::*;
use crate::replay_capsules::status_fact;
use std::collections::BTreeSet;

#[derive(serde::Deserialize)]
struct MySessionStatusParams {
    title: String,
}

pub(in crate::daemon::server) fn rpc_my_session(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let rec = resolve_caller(state, params, "my session")?;
    let roots = state.with_store(super::who::root_channels)?;
    let instance = state.session_instance(&rec);
    let expanded_workspaces = state.with_store(|store| {
        store
            .list_session_joined_channels(&rec.session_id)
            .unwrap_or_default()
            .into_iter()
            .map(|(channel, _)| {
                store
                    .root_channel_of(&channel)
                    .ok()
                    .flatten()
                    .unwrap_or(channel)
            })
            .collect::<BTreeSet<_>>()
    });
    let host = state.host.clone();
    let backend_pubkey = state.backend_pubkey().unwrap_or_default();
    let fabric = state.with_store(|store| {
        crate::who_view::render_agent_who(
            store,
            crate::who_view::AgentWhoInput {
                roots: &roots,
                self_name: &instance.display_slug(),
                self_pubkey: &instance.pubkey,
                local_host: &host,
                backend_pubkey: &backend_pubkey,
                now: now_secs(),
                expanded_workspaces: &expanded_workspaces,
            },
        )
    });
    Ok(serde_json::json!({ "fabric": fabric }))
}

pub(in crate::daemon::server) async fn rpc_my_session_status(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: MySessionStatusParams =
        serde_json::from_value(params.clone()).context("my session status params")?;
    let title = crate::work_topic::normalize(&p.title)?;
    let rec = resolve_caller(state, params, "my session status")?;
    let set_at = now_secs();
    state.with_store(|s| s.set_session_work_topic(&rec.session_id, &title, set_at))?;
    let keys = state.session_signing_keys(&rec.session_id)?;
    crate::status_seam::drive(
        &state.status,
        state.fabric_provider(),
        &keys,
        &state.store,
        &state.outbox,
        crate::status_seam::DriveMeta {
            trigger: "manual_title",
            window_hash: None,
            replay_fact: Some(status_fact!(title, rec.session_id, title, set_at)),
        },
        |r| r.on_title_set(&rec.session_id, &title, set_at),
    )
    .await;
    state.outbox_notify.notify_waiters();
    Ok(serde_json::json!({
        "session_id": rec.session_id,
        "title": title,
        "distill_paused_until": set_at.saturating_add(crate::work_topic::DISTILL_PAUSE_SECS),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RegisterSession;
    use std::collections::BTreeSet;

    #[tokio::test]
    async fn briefing_is_read_only_and_expands_only_the_exact_sessions_workspaces() {
        let state = DaemonState::new_for_test().await;
        let session_id = state.with_store(|s| {
            s.upsert_channel("alpha", "alpha", "Alpha", "", 1).unwrap();
            s.upsert_channel("beta", "beta", "Beta", "", 1).unwrap();
            s.upsert_profile("pk", "codex", "codex", "test-host", false, 1)
                .unwrap();
            s.upsert_channel_member("alpha", "pk", "member", 1).unwrap();
            s.upsert_channel_member("beta", "pk", "member", 1).unwrap();
            s.register_session(&RegisterSession {
                harness: "codex".into(),
                external_id_kind: "pty_session".into(),
                external_id: "pty-briefing".into(),
                agent_pubkey: "pk".into(),
                agent_slug: "codex".into(),
                channel_h: "alpha".into(),
                child_pid: Some(42),
                transcript_path: None,
                resume_id: String::new(),
                now: 10,
            })
            .unwrap()
        });

        let first = rpc_my_session(
            &state,
            &serde_json::json!({
                "pty_session": "pty-briefing"
            }),
        )
        .unwrap();
        let first = first["fabric"].as_str().expect("agent briefing");
        assert!(first.contains("<self name=\"@codex\""), "{first}");
        assert!(first.contains("<agents>"), "{first}");
        assert!(first.contains(
            "<workspace name=\"alpha\" channel=\"alpha\" about=\"Alpha\" members=\"1\">"
        ));
        assert!(first
            .contains("<workspace name=\"beta\" channel=\"beta\" about=\"Beta\" members=\"1\" />"));

        state.with_store(|s| s.join_session_channel(&session_id, "beta", 20).unwrap());
        let second = rpc_my_session(
            &state,
            &serde_json::json!({
                "pty_session": "pty-briefing"
            }),
        )
        .unwrap();
        let second = second["fabric"].as_str().expect("agent briefing");
        assert!(second
            .contains("<workspace name=\"beta\" channel=\"beta\" about=\"Beta\" members=\"1\">"));

        let seen_cursor = state.with_store(|s| {
            s.get_session(&session_id)
                .unwrap()
                .expect("session row")
                .seen_cursor
        });
        assert_eq!(seen_cursor, 0, "pure read must not advance hook cursor");
    }

    #[tokio::test]
    async fn stores_and_publishes_title_for_the_exact_caller_session() {
        let state = DaemonState::new_for_test().await;
        let session_id = state.with_store(|s| {
            s.register_session(&RegisterSession {
                harness: "codex".into(),
                external_id_kind: "pty_session".into(),
                external_id: "pty-1".into(),
                agent_pubkey: "pk".into(),
                agent_slug: "codex".into(),
                channel_h: "root".into(),
                child_pid: None,
                transcript_path: None,
                resume_id: String::new(),
                now: 1,
            })
            .unwrap()
        });
        {
            let mut status = state.status.lock().unwrap();
            let out = status
                .on_session_started(
                    &session_id,
                    "test-host",
                    "codex",
                    "pk",
                    ".",
                    BTreeSet::from(["root".to_string()]),
                    true,
                    "",
                    "checking logs",
                    1,
                )
                .unwrap();
            assert_eq!(out.effects.len(), 1);
        }

        let response = rpc_my_session_status(
            &state,
            &serde_json::json!({
                "pty_session": "pty-1",
                "title": "Researching MCP improvements around resource allocation",
            }),
        )
        .await
        .unwrap();

        assert_eq!(response["session_id"], session_id);
        let rec = state.with_store(|s| s.get_session(&session_id).unwrap().unwrap());
        assert_eq!(
            rec.work_topic,
            "Researching MCP improvements around resource allocation"
        );
        assert!(rec.work_topic_set_at > 0);
        assert_eq!(
            rec.title,
            "Researching MCP improvements around resource allocation"
        );

        let rows = state.with_store(|s| s.peek_outbox(10, u64::MAX).unwrap());
        assert_eq!(rows.len(), 1);
        let event: serde_json::Value = serde_json::from_str(&rows[0].event_json).unwrap();
        assert_eq!(event["kind"].as_i64(), Some(30315));
        assert!(event["tags"].as_array().unwrap().iter().any(|tag| {
            tag.as_array().is_some_and(|parts| {
                parts.first().and_then(|v| v.as_str()) == Some("title")
                    && parts.get(1).and_then(|v| v.as_str())
                        == Some("Researching MCP improvements around resource allocation")
            })
        }));
    }

    #[tokio::test]
    async fn rejects_topics_over_fifteen_words() {
        let state = DaemonState::new_for_test().await;
        let title = vec!["word"; crate::work_topic::MAX_WORDS + 1].join(" ");
        let err = rpc_my_session_status(&state, &serde_json::json!({ "title": title }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("at most"));
    }
}
