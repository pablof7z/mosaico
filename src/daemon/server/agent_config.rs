//! Daemon-owned mutation boundary for durable agent identity configuration.

use anyhow::{Context, Result};
use serde::Deserialize;

use super::state::AgentConfigState;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentSaveParams {
    slug: String,
    harness: String,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    per_session_key: Option<bool>,
}

pub(super) fn rpc_agent_save(
    state: &AgentConfigState,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let params: AgentSaveParams =
        serde_json::from_value(params.clone()).context("agent_save params")?;
    let (identity, created) = state.mutate(|| {
        crate::identity::save_local_agent(
            &crate::config::mosaico_home(),
            &params.slug,
            crate::identity::LocalAgentUpdate {
                harness: params.harness,
                profile: params.profile,
                per_session_key: params.per_session_key,
                byline: None,
            },
            crate::util::now_secs(),
        )
    })?;
    Ok(serde_json::json!({
        "created": created,
        "slug": identity.slug,
        "harness": identity.harness,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentRemoveParams {
    slug: String,
}

pub(super) fn rpc_agent_remove(
    state: &AgentConfigState,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let params: AgentRemoveParams =
        serde_json::from_value(params.clone()).context("agent_remove params")?;
    let removed = state.mutate(|| {
        crate::identity::remove_local_agent(&crate::config::mosaico_home(), &params.slug)
    })?;
    Ok(serde_json::json!({ "removed": removed }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::EnvGuard;
    use std::sync::{Arc, Barrier};

    fn isolated_home() -> (tempfile::TempDir, EnvGuard) {
        let root = tempfile::tempdir().unwrap();
        let mosaico_home = root.path().join(".mosaico");
        let mut env = EnvGuard::set("HOME", root.path());
        env.set_var("MOSAICO_HOME", &mosaico_home);
        env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
        (root, env)
    }

    #[test]
    fn daemon_rpc_owns_agent_record_save_and_remove() {
        let (root, _env) = isolated_home();
        let mosaico_home = root.path().join(".mosaico");
        let state = AgentConfigState::new();

        let saved = rpc_agent_save(
            &state,
            &serde_json::json!({
                "slug": "writer",
                "harness": "codex-pty",
                "profile": "reviewer",
                "per_session_key": true,
            }),
        )
        .unwrap();
        assert_eq!(saved["created"], true);
        assert_eq!(saved["harness"], "codex-pty");
        assert!(mosaico_home.join("agents/writer.json").is_file());

        let removed = rpc_agent_remove(&state, &serde_json::json!({ "slug": "writer" })).unwrap();
        assert_eq!(removed["removed"], true);
        assert!(!mosaico_home.join("agents/writer.json").exists());
    }

    #[test]
    fn updating_a_corrupt_record_uses_the_canonical_parser() {
        let (root, _env) = isolated_home();
        let mosaico_home = root.path().join(".mosaico");
        std::fs::create_dir_all(mosaico_home.join("agents")).unwrap();
        std::fs::write(mosaico_home.join("agents/writer.json"), "not json").unwrap();
        let state = AgentConfigState::new();

        let error = rpc_agent_save(
            &state,
            &serde_json::json!({
                "slug": "writer",
                "harness": "codex-pty",
            }),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("parsing agent record"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn concurrent_saves_linearize_one_create_and_one_update() {
        let (root, _env) = isolated_home();
        let state = Arc::new(AgentConfigState::new());
        let start = Arc::new(Barrier::new(3));
        let calls = ["codex-pty", "claude-acp"].map(|harness| {
            let state = Arc::clone(&state);
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                rpc_agent_save(
                    &state,
                    &serde_json::json!({ "slug": "writer", "harness": harness }),
                )
            })
        });

        start.wait();
        let saved = calls.map(|call| call.join().unwrap().unwrap());
        assert_eq!(
            saved
                .iter()
                .filter(|value| value["created"] == true)
                .count(),
            1
        );
        let path = root.path().join(".mosaico/agents/writer.json");
        let stored: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(stored["slug"], "writer");
        assert!(["codex-pty", "claude-acp"].contains(&stored["harness"].as_str().unwrap()));
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn concurrent_save_and_remove_produce_a_linearized_final_state() {
        let (root, _env) = isolated_home();
        let state = Arc::new(AgentConfigState::new());
        rpc_agent_save(
            &state,
            &serde_json::json!({ "slug": "writer", "harness": "codex-pty" }),
        )
        .unwrap();
        let start = Arc::new(Barrier::new(3));
        let save = {
            let state = Arc::clone(&state);
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                rpc_agent_save(
                    &state,
                    &serde_json::json!({ "slug": "writer", "harness": "claude-acp" }),
                )
            })
        };
        let remove = {
            let state = Arc::clone(&state);
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                rpc_agent_remove(&state, &serde_json::json!({ "slug": "writer" }))
            })
        };

        start.wait();
        let saved = save.join().unwrap().unwrap();
        let removed = remove.join().unwrap().unwrap();
        assert_eq!(removed["removed"], true);
        let path = root.path().join(".mosaico/agents/writer.json");
        assert_eq!(path.exists(), saved["created"] == true);
        if path.exists() {
            let stored: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(stored["harness"], "claude-acp");
        }
        assert!(!path.with_extension("json.tmp").exists());
    }
}
