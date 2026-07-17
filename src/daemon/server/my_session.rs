use super::channel_membership_rpc::resolve_caller;
use super::*;
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
    let headless = state.with_store(|store| crate::session_host::session_is_headless(store, &rec));
    let expanded_workspaces = state.with_store(|store| -> Result<BTreeSet<String>> {
        store
            .list_session_joined_channels(&rec.pubkey)?
            .into_iter()
            .map(|(channel, _)| {
                crate::daemon::workspace_path::WorkspacePathResolver::new(store)
                    .root_for_channel(&channel)
            })
            .collect()
    })?;
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
                headless,
                expanded_workspaces: &expanded_workspaces,
            },
        )
    })?;
    Ok(serde_json::json!({ "fabric": fabric }))
}

pub(in crate::daemon::server) async fn rpc_my_session_status(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: MySessionStatusParams =
        serde_json::from_value(params.clone()).context("my session status params")?;
    let title = crate::session_title::normalize(&p.title)?;
    let rec = resolve_caller(state, params, "my session status")?;
    let set_at = now_secs();
    state.with_store(|s| s.set_session_title(&rec.pubkey, &title))?;
    let keys = state.session_signing_keys(&rec.pubkey)?;
    crate::status_seam::drive(
        &state.reconcilers.status,
        state.fabric_provider(),
        &keys,
        &state.store,
        crate::status_seam::DriveMeta {
            trigger: "manual_title",
        },
        |r| r.on_title_set(&rec.pubkey, &title, set_at),
    )
    .await;
    Ok(serde_json::json!({ "title": title }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RegisterSession;
    use std::collections::BTreeSet;

    #[tokio::test]
    async fn briefing_is_read_only_and_expands_only_the_exact_sessions_workspaces() {
        let state = DaemonState::new_for_test().await;
        let pubkey = state.with_store(|s| {
            s.upsert_channel("alpha", "alpha", "Alpha", "", 1).unwrap();
            s.upsert_channel("beta", "beta", "Beta", "", 1).unwrap();
            s.upsert_profile("pk", "codex", "codex", "test-host", false, 1)
                .unwrap();
            s.upsert_channel_member("alpha", "pk", "member", 1).unwrap();
            s.upsert_channel_member("beta", "pk", "member", 1).unwrap();
            s.reserve_session_with_facts(
                &RegisterSession {
                    pubkey: "pk".into(),
                    observed_harness: "codex".into(),
                    agent_slug: "codex".into(),
                    channel_h: "alpha".into(),
                    child_pid: Some(42),
                    transcript_path: None,
                    now: 10,
                },
                &crate::state::AdmittedRuntimeFacts {
                    observed_harness: "codex".into(),
                    claimed_harness: String::new(),
                    bundle: "codex-pty".into(),
                    transport: "pty".into(),
                    endpoint_provenance: "launch".into(),
                },
            )
            .unwrap();
            s.put_session_locator("codex", crate::state::LOCATOR_PTY, "pty-briefing", "pk", 10)
                .unwrap();
            "pk".to_string()
        });

        let first = rpc_my_session(
            &state,
            &serde_json::json!({
                "pty_session": "pty-briefing",
                "harness": "codex"
            }),
        )
        .unwrap();
        let first = first["fabric"].as_str().expect("agent briefing");
        assert!(first.contains("<self name=\"@codex\""), "{first}");
        assert!(first.contains("headless=\"on\""), "{first}");
        assert!(first.contains("<agents>"), "{first}");
        assert!(first.contains(
            "<workspace name=\"alpha\" channel=\"alpha\" about=\"Alpha\" members=\"1\">"
        ));
        assert!(first
            .contains("<workspace name=\"beta\" channel=\"beta\" about=\"Beta\" members=\"1\" />"));

        state.with_store(|s| s.join_session_channel(&pubkey, "beta", 20).unwrap());
        let second = rpc_my_session(
            &state,
            &serde_json::json!({
                "pty_session": "pty-briefing",
                "harness": "codex"
            }),
        )
        .unwrap();
        let second = second["fabric"].as_str().expect("agent briefing");
        assert!(second
            .contains("<workspace name=\"beta\" channel=\"beta\" about=\"Beta\" members=\"1\">"));

        let seen_cursor = state.with_store(|s| {
            s.get_session(&pubkey)
                .unwrap()
                .expect("session row")
                .seen_cursor
        });
        assert_eq!(seen_cursor, 0, "pure read must not advance hook cursor");
    }

    #[tokio::test]
    async fn stores_and_publishes_title_for_the_exact_caller_session() {
        let state = DaemonState::new_for_test().await;
        let signer_salt = crate::identity::new_session_signer_salt();
        let keys = crate::identity::derive_session_keys(
            state.management_keys().unwrap().secret_key(),
            &signer_salt,
        )
        .unwrap();
        let pubkey = keys.public_key().to_hex();
        state.with_store(|s| {
            s.reserve_session_with_facts(
                &RegisterSession {
                    pubkey: pubkey.clone(),
                    observed_harness: "codex".into(),
                    agent_slug: "codex".into(),
                    channel_h: "root".into(),
                    child_pid: None,
                    transcript_path: None,
                    now: 1,
                },
                &crate::state::AdmittedRuntimeFacts {
                    observed_harness: "codex".into(),
                    claimed_harness: String::new(),
                    bundle: "codex-pty".into(),
                    transport: "pty".into(),
                    endpoint_provenance: "launch".into(),
                },
            )
            .unwrap();
            s.put_session_locator("codex", crate::state::LOCATOR_PTY, "pty-1", &pubkey, 1)
                .unwrap();
        });
        {
            let mut status = state.reconcilers.status.lock().unwrap();
            let out = status.on_session_started(
                &pubkey,
                "test-host",
                "codex",
                ".",
                BTreeSet::from(["root".to_string()]),
                true,
                true,
                "",
                1,
            );
            assert_eq!(out.effects.len(), 1);
        }
        state.with_store(|s| s.bind_session_signer(&pubkey, &signer_salt).unwrap());

        let response = rpc_my_session_status(
            &state,
            &serde_json::json!({
                "pty_session": "pty-1",
                "harness": "codex",
                "title": "Researching MCP improvements around resource allocation",
            }),
        )
        .await
        .unwrap();

        assert!(response.get("session_id").is_none());
        assert_eq!(
            response["title"],
            "Researching MCP improvements around resource allocation"
        );
        let rec = state.with_store(|s| s.get_session(&pubkey).unwrap().unwrap());
        assert_eq!(
            rec.title,
            "Researching MCP improvements around resource allocation"
        );
    }

    #[tokio::test]
    async fn rejects_titles_over_fifteen_words() {
        let state = DaemonState::new_for_test().await;
        let title = vec!["word"; crate::session_title::MAX_WORDS + 1].join(" ");
        let err = rpc_my_session_status(&state, &serde_json::json!({ "title": title }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("at most"));
    }
}
