//! Host agent inventory for backend-addressed management commands.

use super::super::DaemonState;
use anyhow::Result;
use std::sync::Arc;

pub(super) fn list_agents(state: &Arc<DaemonState>) -> Result<String> {
    let snapshot = super::super::backend_profile::backend_profile_snapshot(state)?;
    if snapshot.agents.is_empty() {
        return Ok(format!("mgmt ok: no agents known on {}", state.host()));
    }
    let mut lines = vec![format!(
        "mgmt ok: {} agent(s) on {}",
        snapshot.agents.len(),
        state.host()
    )];
    for agent in snapshot.agents {
        let description = crate::agent_about::for_injection(&agent.about);
        if description.is_empty() {
            lines.push(format!("- {}", agent.slug));
        } else {
            lines.push(format!("- {}: {description}", agent.slug));
        }
    }
    for failure in snapshot.failures {
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

    fn write_executable(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt as _;

        write(path, "#!/bin/sh\n");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
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
            &codex_home.join("agents/writer.toml"),
            "name='writer'\ndescription='Writes'\ndeveloper_instructions='Write'",
        );
        let verbose_description = "é".repeat(crate::agent_about::AGENT_ABOUT_MAX_CHARS + 1);
        write(
            &codex_home.join("agents/verbose.toml"),
            &format!(
                "name='verbose'\ndescription='{verbose_description}'\ndeveloper_instructions='Write'"
            ),
        );
        write(
            &root.path().join(".claude/agents/writer.md"),
            "---\nname: writer\ndescription: Writes\n---\nWrite",
        );
        for executable in ["claude", "codex"] {
            write_executable(&root.path().join(".local/bin").join(executable));
        }
        write_executable(&root.path().join(".opencode/bin/opencode"));
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
        assert!(!listed.contains("available"), "{listed}");
        let verbose_line = listed
            .lines()
            .find(|line| line.starts_with("- verbose: "))
            .unwrap_or_else(|| panic!("missing verbose agent in {listed}"));
        let description = verbose_line.strip_prefix("- verbose: ").unwrap();
        assert_eq!(
            description.chars().count(),
            crate::agent_about::AGENT_ABOUT_MAX_CHARS
        );
        assert!(description.ends_with('…'), "{description}");
        assert_eq!(
            description
                .chars()
                .take(crate::agent_about::AGENT_ABOUT_MAX_CHARS - 1)
                .collect::<String>(),
            "é".repeat(crate::agent_about::AGENT_ABOUT_MAX_CHARS - 1)
        );
    }
}
