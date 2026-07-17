//! Agent roster listing for backend-addressed management commands.

use super::super::DaemonState;
use anyhow::Result;
use std::sync::Arc;

pub(super) fn list_agents(state: &Arc<DaemonState>) -> Result<String> {
    let now = crate::util::now_secs();
    let (agents, failures) = super::super::agent_roster::capability_advertisements(state)?;
    if agents.is_empty() {
        return Ok(format!("mgmt ok: no agents known on {}", state.host));
    }
    let mut lines = vec![format!(
        "mgmt ok: {} agent(s) on {}",
        agents.len(),
        state.host
    )];
    for agent in agents {
        let criteria = agent.use_criteria.trim();
        let age = (agent.available_since > 0)
            .then(|| crate::util::relative_time(agent.available_since, now));
        if criteria.is_empty() && age.is_none() {
            lines.push(format!("- {}", agent.slug));
        } else if criteria.is_empty() {
            let age = age.as_deref().unwrap_or_default();
            lines.push(format!("- {} (available {age})", agent.slug));
        } else if age.is_none() {
            lines.push(format!("- {}: {criteria}", agent.slug));
        } else {
            let age = age.as_deref().unwrap_or_default();
            lines.push(format!("- {}: {criteria} (available {age})", agent.slug));
        }
    }
    for failure in failures {
        tracing::warn!(error = %failure, "agent inventory entry is unavailable");
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::EnvGuard;

    fn write(path: &std::path::Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    #[tokio::test]
    async fn lists_bare_harnesses_and_expanded_profile_conflicts() {
        let root = tempfile::tempdir().unwrap();
        let mosaico_home = root.path().join("mosaico");
        let codex_home = root.path().join(".codex");
        let mut env = EnvGuard::set("HOME", root.path());
        env.set_var("MOSAICO_HOME", &mosaico_home);
        env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
        env.set_var("CODEX_HOME", &codex_home);
        write(
            &mosaico_home.join("harnesses.json"),
            r#"{
              "claude-pty":{"harness":"claude","transport":"pty"},
              "codex-pty":{"harness":"codex","transport":"pty"},
              "opencode-pty":{"harness":"opencode","transport":"pty"}
            }"#,
        );
        write(
            &codex_home.join("agents/writer.toml"),
            "name='writer'\ndescription='Writes'\ndeveloper_instructions='Write'",
        );
        write(
            &root.path().join(".claude/agents/writer.md"),
            "---\nname: writer\ndescription: Writes\n---\nWrite",
        );
        std::fs::create_dir_all(root.path().join(".config/opencode")).unwrap();
        let state = DaemonState::new_for_test().await;
        state.refresh_agent_catalog().unwrap();

        let listed = list_agents(&state).unwrap();

        for slug in [
            "- claude",
            "- codex",
            "- opencode",
            "- writer-claude",
            "- writer-codex",
        ] {
            assert!(listed.contains(slug), "missing {slug:?} in {listed}");
        }
    }
}
