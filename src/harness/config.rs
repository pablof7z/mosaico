//! `~/.mosaico/harnesses.json` loader + serde.
//!
//! The file is a map of **bundle name -> bundle spec**. A bundle is the
//! user-facing name you spawn (`codex-acp`, `planner`, …); it binds a `harness`
//! (which CLI) to a `transport` (how mosaico drives it) plus operational args.
//! Missing file => empty map; malformed JSON => hard error. There are no
//! built-in bundle fallbacks.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::session::Harness;

/// Whole-file shape: bundle name -> bundle spec.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HarnessesConfig {
    pub bundles: BTreeMap<String, HarnessBundle>,
}

/// One bundle: which CLI, how we drive it, and opaque tuning.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessBundle {
    /// Which underlying CLI. Parsed via `Harness::from_str` so `"claude"` and
    /// `"claude-code"` both resolve; `Harness::Unknown` is rejected.
    #[serde(with = "harness_serde")]
    pub harness: Harness,
    /// How mosaico drives that CLI.
    pub transport: Transport,
    /// Bundle-owned operational flags appended to the code-owned driver argv.
    /// The executable itself is never configurable JSON.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

/// How mosaico drives a CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Transport {
    /// Interactive terminal via the existing portable-pty supervisor.
    Pty,
    /// ACP: JSON-RPC 2.0 over stdio (OpenCode native, Claude via adapter).
    Acp,
    /// Codex `app-server`: its own JSON-RPC dialect.
    AppServer,
    /// One-shot run-to-exit (`claude -p`, `codex exec`, `opencode run`).
    HeadlessExec,
}

impl Transport {
    pub fn as_str(&self) -> &'static str {
        match self {
            Transport::Pty => "pty",
            Transport::Acp => "acp",
            Transport::AppServer => "app-server",
            Transport::HeadlessExec => "headless-exec",
        }
    }
}

impl HarnessesConfig {
    /// Load `<mosaico_home>/harnesses.json`. Absent file => empty map; a launch
    /// still fails unless its agent-selected bundle is explicitly configured.
    pub fn load() -> anyhow::Result<Self> {
        let path = crate::config::mosaico_home().join("harnesses.json");
        Self::load_from(&path)
    }

    pub fn load_from(path: &std::path::Path) -> anyhow::Result<Self> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => {
                return Err(e).map_err(|e| {
                    anyhow::anyhow!("reading harnesses config {}: {e}", path.display())
                })
            }
        };
        serde_json::from_slice(&bytes)
            .map_err(|e| anyhow::anyhow!("parsing harnesses config {}: {e}", path.display()))
    }

    pub fn get(&self, bundle: &str) -> Option<&HarnessBundle> {
        self.bundles.get(bundle)
    }
}

/// (De)serialize `Harness` through its existing `from_str`/`as_str` so the
/// config axis stays independent of the enum's Rust spelling and rejects
/// unknown harnesses loudly.
mod harness_serde {
    use super::Harness;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(h: &Harness, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(h.as_str())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Harness, D::Error> {
        let raw = String::deserialize(d)?;
        match Harness::from_str(&raw) {
            Harness::Unknown => Err(serde::de::Error::custom(format!(
                "unknown harness {raw:?} (expected claude-code|codex|opencode|grok)"
            ))),
            h => Ok(h),
        }
    }
}
