//! Daemon-owned mutation boundary for durable agent identity configuration.

use anyhow::{Context, Result};
use serde::Deserialize;

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

pub(super) fn rpc_agent_save(params: &serde_json::Value) -> Result<serde_json::Value> {
    let params: AgentSaveParams =
        serde_json::from_value(params.clone()).context("agent_save params")?;
    let (identity, created) = crate::identity::save_local_agent(
        &crate::config::mosaico_home(),
        &params.slug,
        crate::identity::LocalAgentUpdate {
            harness: params.harness,
            profile: params.profile,
            per_session_key: params.per_session_key,
            byline: None,
        },
        crate::util::now_secs(),
    )?;
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

pub(super) fn rpc_agent_remove(params: &serde_json::Value) -> Result<serde_json::Value> {
    let params: AgentRemoveParams =
        serde_json::from_value(params.clone()).context("agent_remove params")?;
    let removed =
        crate::identity::remove_local_agent(&crate::config::mosaico_home(), &params.slug)?;
    Ok(serde_json::json!({ "removed": removed }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::EnvGuard;

    #[test]
    fn daemon_rpc_owns_agent_record_save_and_remove() {
        let root = tempfile::tempdir().unwrap();
        let mosaico_home = root.path().join(".mosaico");
        let mut env = EnvGuard::set("HOME", root.path());
        env.set_var("MOSAICO_HOME", &mosaico_home);
        env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");

        let saved = rpc_agent_save(&serde_json::json!({
            "slug": "writer",
            "harness": "codex-pty",
            "profile": "reviewer",
            "per_session_key": true,
        }))
        .unwrap();
        assert_eq!(saved["created"], true);
        assert_eq!(saved["harness"], "codex-pty");
        assert!(mosaico_home.join("agents/writer.json").is_file());

        let removed = rpc_agent_remove(&serde_json::json!({ "slug": "writer" })).unwrap();
        assert_eq!(removed["removed"], true);
        assert!(!mosaico_home.join("agents/writer.json").exists());
    }

    #[test]
    fn updating_a_corrupt_record_uses_the_canonical_parser() {
        let root = tempfile::tempdir().unwrap();
        let mosaico_home = root.path().join(".mosaico");
        std::fs::create_dir_all(mosaico_home.join("agents")).unwrap();
        std::fs::write(mosaico_home.join("agents/writer.json"), "not json").unwrap();
        let mut env = EnvGuard::set("HOME", root.path());
        env.set_var("MOSAICO_HOME", &mosaico_home);
        env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");

        let error = rpc_agent_save(&serde_json::json!({
            "slug": "writer",
            "harness": "codex-pty",
        }))
        .unwrap_err();
        assert!(
            error.to_string().contains("parsing agent record"),
            "unexpected error: {error:#}"
        );
    }
}
