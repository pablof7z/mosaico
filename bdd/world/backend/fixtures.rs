//! Native profile records and deterministic executable shims.

use std::os::unix::fs::PermissionsExt as _;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use nostr::Keys;

use super::Backend;

impl Backend {
    pub fn install_claude_profile_agent(&self, profile: &str, agent: &str) -> Result<()> {
        let profiles = self.home.join(".claude/agents");
        std::fs::create_dir_all(&profiles)?;
        let document = format!(
            "---\nname: {profile}\ndescription: BDD fixture profile\n---\nReview carefully.\n"
        );
        std::fs::write(profiles.join(format!("{profile}.md")), document)?;
        let presets = serde_json::json!({
            "unrestricted": {
                "claude-code": {"pty": ["--dangerously-skip-permissions"]}
            }
        });
        std::fs::write(
            self.mosaico_home.join("presets.json"),
            serde_json::to_vec_pretty(&presets)?,
        )?;
        let result = self.run(
            &[
                "agents",
                "add",
                agent,
                "--harness",
                "claude-code",
                "--preset",
                "unrestricted",
                "--profile",
                profile,
            ],
            None,
            Duration::from_secs(15),
        )?;
        anyhow::ensure!(
            result.success(),
            "install profile agent failed: {}",
            result.combined()
        );
        Ok(())
    }

    pub fn make_agent_stable(&self, agent: &str) -> Result<String> {
        let path = self
            .mosaico_home
            .join("agents")
            .join(format!("{agent}.json"));
        let mut record: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        let secret = format!("{:064x}", 700);
        let public = Keys::parse(&secret)?.public_key().to_hex();
        record["perSessionKey"] = serde_json::json!(false);
        record["secret_key"] = serde_json::json!(secret);
        record["public_key"] = serde_json::json!(public);
        std::fs::write(path, serde_json::to_vec_pretty(&record)?)?;
        // The fixture edits the durable identity document directly. Make the
        // daemon rediscover that current document before a relay mention can
        // race activation against its older in-memory catalog.
        let refreshed = self.run(&["agents", "list"], None, Duration::from_secs(15))?;
        anyhow::ensure!(
            refreshed.success() && refreshed.stdout.contains(agent),
            "stable agent catalog refresh failed: {}",
            refreshed.combined()
        );
        Ok(public)
    }

    pub fn harness_pubkey(&self) -> Result<String> {
        let path = self.mosaico_home.join("harness-pubkey");
        wait_for_file(&path);
        Ok(std::fs::read_to_string(&path)
            .with_context(|| format!("read harness pubkey capture {}", path.display()))?
            .trim()
            .to_string())
    }

    pub fn harness_input(&self) -> String {
        std::fs::read_to_string(self.mosaico_home.join("harness-input")).unwrap_or_default()
    }

    pub(super) fn install_shims(&self) -> Result<()> {
        let script = r#"#!/bin/sh
if [ "${1:-}" = "--version" ]; then
  echo 1.99.0
  exit 0
fi
printf '%s\n' "${MOSAICO_PUBKEY:-}" > "${MOSAICO_HOME}/harness-pubkey"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "${MOSAICO_HOME}/harness-input"
done
"#;
        for bin in [self.home.join("bin"), self.home.join(".local/bin")] {
            std::fs::create_dir_all(&bin)?;
            for name in [
                "claude", "codex", "opencode", "grok", "goose", "hermes", "kimi",
            ] {
                write_executable(&bin.join(name), script)?;
            }
        }
        Ok(())
    }
}

fn wait_for_file(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn write_executable(path: &std::path::Path, body: &str) -> Result<()> {
    std::fs::write(path, body)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}
