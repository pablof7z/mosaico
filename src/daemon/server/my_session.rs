use super::channel_membership_rpc::resolve_caller;
use super::*;

#[derive(serde::Deserialize)]
struct MySessionStatusParams {
    title: String,
}

pub(in crate::daemon::server) fn rpc_my_session(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let rec = resolve_caller(state, params, "my session")?;
    let instance = state.session_instance(&rec);
    let host = state.host().clone();
    let backend_pubkey = state.backend_pubkey().unwrap_or_default();
    let fabric = state.with_store(|store| {
        crate::fabric_context::render_full_session_state(
            store,
            &rec,
            &instance.display_slug(),
            &backend_pubkey,
            &host,
            now_secs(),
            super::session_delivery::extension_delivery_live(state, &rec),
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
    state.with_store(|s| s.set_session_title(&rec.pubkey, &title))?;
    super::presence::reconcile_generation(
        state,
        &rec.pubkey,
        rec.runtime_generation,
        "manual_title",
        None,
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
            s.install_test_nmp_group_delivery(crate::state::TestGroupDelivery::new([
                crate::state::TestGroup::new("alpha")
                    .metadata("alpha", "Alpha", "", 1)
                    .admins(Vec::new())
                    .members(vec!["pk".into()]),
                crate::state::TestGroup::new("beta")
                    .metadata("beta", "Beta", "", 1)
                    .admins(Vec::new())
                    .members(vec!["pk".into()]),
            ]));
            s.install_test_nmp_relay_delivery(crate::state::TestRelayDelivery::new().profiles([
                crate::state::Profile {
                    pubkey: "pk".into(),
                    name: "codex".into(),
                    slug: "codex".into(),
                    agent_slug: String::new(),
                    host: "test-host".into(),
                    is_backend: false,
                    agents: Vec::new(),
                    workspaces: Vec::new(),
                    updated_at: 1,
                },
            ]));
            let generation = s
                .reserve_session_with_facts(
                    &RegisterSession {
                        pubkey: "pk".into(),
                        observed_harness: "codex".into(),
                        agent_slug: "codex".into(),
                        launch_channel_h: "alpha".into(),
                        work_root: "alpha".into(),
                        child_pid: Some(42),
                        now: 10,
                    },
                    &crate::state::AdmittedRuntimeFacts {
                        observed_harness: "codex".into(),
                        claimed_harness: String::new(),
                        preset: String::new(),
                        transport: "pty".into(),
                        endpoint_provenance: "launch".into(),
                    },
                )
                .unwrap();
            s.apply_session_presentation_edge(
                "pk",
                generation,
                1,
                crate::state::PresentationState::Headless,
                10,
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
        assert!(!first.contains("<hosts>"), "{first}");
        assert!(!first.contains("<workspace"), "{first}");
        assert!(
            first.contains("<channel name=\"#alpha\" about=\"Alpha\" agents=\"1\">")
                && first.contains("<agent name=\"@codex\""),
            "{first}"
        );
        assert!(
            first.contains("<channel name=\"#beta\" about=\"Beta\" agents=\"1\" />"),
            "{first}"
        );

        state.with_store(|s| s.grant_session_route(&pubkey, "beta", 20).unwrap());
        let second = rpc_my_session(
            &state,
            &serde_json::json!({
                "pty_session": "pty-briefing",
                "harness": "codex"
            }),
        )
        .unwrap();
        let second = second["fabric"].as_str().expect("agent briefing");
        assert!(!second.contains("<workspace"), "{second}");
        assert!(
            second.contains("<channel name=\"#beta\" about=\"Beta\" agents=\"1\">"),
            "{second}"
        );

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
                    launch_channel_h: "root".into(),
                    work_root: "root".into(),
                    child_pid: None,
                    now: 1,
                },
                &crate::state::AdmittedRuntimeFacts {
                    observed_harness: "codex".into(),
                    claimed_harness: String::new(),
                    preset: String::new(),
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
            let out = status.open(
                &pubkey,
                1,
                crate::reconcile::PresenceSnapshot {
                    host: "test-host".into(),
                    workspace: "root".into(),
                    slug: "codex".into(),
                    rel_cwd: ".".into(),
                    dispatch_event: None,
                    projection: crate::reconcile::PresenceProjection {
                        channels: BTreeSet::from(["root".to_string()]),
                        branch: "feat/context".into(),
                        state: crate::session_state::SessionState::Working,
                        state_since: 1,
                        title: String::new(),
                    },
                },
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
