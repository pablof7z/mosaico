//! Reusable, explicitly selected launch arguments.
//!
//! `presets.json` is preset-first: preset -> harness -> transport -> args.
//! Presets customize an already-selected driver. They never select a harness,
//! executable, or transport.

use super::Transport;
use crate::session::Harness;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PresetsConfig {
    presets: BTreeMap<String, BTreeMap<String, HarnessPreset>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessPreset {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pty: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    acp: Vec<String>,
    #[serde(default, rename = "app-server", skip_serializing_if = "Vec::is_empty")]
    app_server: Vec<String>,
    #[serde(default, rename = "pi-rpc", skip_serializing_if = "Vec::is_empty")]
    pi_rpc: Vec<String>,
}

impl HarnessPreset {
    fn args(&self, transport: Transport) -> &[String] {
        match transport {
            Transport::Pty => &self.pty,
            Transport::Acp => &self.acp,
            Transport::AppServer => &self.app_server,
            Transport::PiRpc => &self.pi_rpc,
        }
    }
}

impl PresetsConfig {
    pub fn load() -> Result<Self> {
        Self::load_from(&crate::config::mosaico_home().join("presets.json"))
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
        };
        let config: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing presets config {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        for (preset, harnesses) in &self.presets {
            if preset.trim().is_empty() {
                anyhow::bail!("preset name must not be empty");
            }
            for (harness, realization) in harnesses {
                let parsed = Harness::from_str(harness);
                if parsed == Harness::Unknown || parsed.as_str() != harness {
                    anyhow::bail!("preset {preset:?} has unknown harness {harness:?}");
                }
                for args in [
                    &realization.pty,
                    &realization.acp,
                    &realization.app_server,
                    &realization.pi_rpc,
                ] {
                    if args.iter().any(|arg| arg.is_empty()) {
                        anyhow::bail!(
                            "preset {preset:?} harness {harness:?} has an empty argument"
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Resolve arguments for an already-selected harness and transport.
    /// A referenced preset and harness realization are required; an omitted
    /// transport cell deliberately contributes no arguments.
    pub fn args(
        &self,
        preset: Option<&str>,
        harness: Harness,
        transport: Transport,
    ) -> Result<Vec<String>> {
        let Some(preset) = preset else {
            return Ok(Vec::new());
        };
        let harnesses = self
            .presets
            .get(preset)
            .with_context(|| format!("no launch preset {preset:?} in presets.json"))?;
        let realization = harnesses.get(harness.as_str()).with_context(|| {
            format!(
                "launch preset {preset:?} has no {} realization",
                harness.as_str()
            )
        })?;
        Ok(realization.args(transport).to_vec())
    }

    pub fn names_for_harness(&self, harness: Harness) -> Vec<String> {
        self.presets
            .iter()
            .filter(|(_, harnesses)| harnesses.contains_key(harness.as_str()))
            .map(|(name, _)| name.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_is_harness_and_transport_specific() {
        let config: PresetsConfig = serde_json::from_str(
            r#"{"unrestricted":{"codex":{"pty":["--yolo"]},"claude-code":{"pty":["--dangerously-skip-permissions"]}}}"#,
        )
        .unwrap();
        config.validate().unwrap();
        assert_eq!(
            config
                .args(Some("unrestricted"), Harness::Codex, Transport::Pty)
                .unwrap(),
            ["--yolo"]
        );
        assert!(config
            .args(Some("unrestricted"), Harness::Codex, Transport::AppServer)
            .unwrap()
            .is_empty());
        assert_eq!(
            config
                .args(Some("unrestricted"), Harness::ClaudeCode, Transport::Pty)
                .unwrap(),
            ["--dangerously-skip-permissions"]
        );
    }

    #[test]
    fn referenced_preset_and_harness_must_exist() {
        let config: PresetsConfig =
            serde_json::from_str(r#"{"unrestricted":{"codex":{"pty":["--yolo"]}}}"#).unwrap();
        assert!(config
            .args(Some("missing"), Harness::Codex, Transport::Pty)
            .unwrap_err()
            .to_string()
            .contains("no launch preset"));
        assert!(config
            .args(Some("unrestricted"), Harness::Hermes, Transport::Pty)
            .unwrap_err()
            .to_string()
            .contains("no hermes realization"));
    }

    #[test]
    fn parser_rejects_unknown_transport_and_harness() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presets.json");
        std::fs::write(&path, r#"{"x":{"codex":{"interactive":["--yolo"]}}}"#).unwrap();
        assert!(PresetsConfig::load_from(&path).is_err());
        std::fs::write(&path, r#"{"x":{"codex-app-server":{"pty":[]}}}"#).unwrap();
        assert!(PresetsConfig::load_from(&path)
            .unwrap_err()
            .to_string()
            .contains("unknown harness"));
    }
}
