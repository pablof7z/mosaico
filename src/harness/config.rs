//! `~/.mosaico/harnesses.json` loader + serde.
//!
//! The file is a map of **bundle name -> bundle spec**. A bundle is the
//! user-facing name you spawn (`codex-app-server`, `planner`, …); it binds a `harness`
//! (which CLI) to a `transport` (how mosaico drives it) plus operational args.
//! Missing file => empty map; malformed JSON => hard error. There are no
//! built-in bundle fallbacks.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::session::Harness;

/// Whole-file shape: bundle name -> bundle spec.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HarnessesConfig {
    pub bundles: BTreeMap<String, HarnessBundle>,
}

/// One bundle: which CLI, how we drive it, and opaque tuning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

impl Transport {
    pub fn as_str(&self) -> &'static str {
        match self {
            Transport::Pty => "pty",
            Transport::Acp => "acp",
            Transport::AppServer => "app-server",
        }
    }
}

impl HarnessesConfig {
    /// Load `<mosaico_home>/harnesses.json`. Absent file => empty map; implicit
    /// launch realization may populate it with canonical hosted policy.
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

    /// Select the sole configured bundle for one hosted matrix cell, or add a
    /// canonical zero-argument bundle when none exists. Tuned bundles remain
    /// authoritative; several candidates are ambiguous and never silently
    /// collapse onto whichever map entry sorts first.
    pub fn resolve_or_create_hosted(
        &mut self,
        harness: Harness,
        transport: Transport,
    ) -> Result<(String, bool)> {
        if crate::harness::driver::lookup(harness, transport).is_none() {
            anyhow::bail!(
                "unsupported hosted harness combination: {} x {}",
                harness.as_str(),
                transport.as_str()
            );
        }
        let candidates = self
            .bundles
            .iter()
            .filter(|(_, bundle)| bundle.harness == harness && bundle.transport == transport)
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        match candidates.as_slice() {
            [name] => Ok((name.clone(), false)),
            [] => self.ensure_bundle(
                &format!("{}-{}", harness.agent_slug(), transport.as_str()),
                HarnessBundle {
                    harness,
                    transport,
                    args: Vec::new(),
                },
            ),
            _ => anyhow::bail!(
                "multiple {} {} bundles are configured ({}); bind an explicit agent to one bundle",
                harness.as_str(),
                transport.as_str(),
                candidates.join(", ")
            ),
        }
    }

    /// Persist the complete bundle map with a temp-file rename.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let body = serde_json::to_string_pretty(self).context("serializing harnesses config")?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, format!("{body}\n"))
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("renaming {} into {}", tmp.display(), path.display()))
    }

    /// Reuse an exactly matching bundle or insert it under a deterministic name.
    ///
    /// A conflicting canonical name gets a numeric suffix. Existing entries are
    /// never changed, so their operational args remain operator-owned.
    pub fn ensure_bundle(
        &mut self,
        canonical_name: &str,
        desired: HarnessBundle,
    ) -> Result<(String, bool)> {
        let canonical_name = canonical_name.trim();
        if canonical_name.is_empty() {
            anyhow::bail!("harness bundle name must not be empty");
        }
        if let Some((name, _)) = self
            .bundles
            .iter()
            .find(|(_, configured)| **configured == desired)
        {
            return Ok((name.clone(), false));
        }

        let name = if !self.bundles.contains_key(canonical_name) {
            canonical_name.to_string()
        } else {
            (2..)
                .map(|suffix| format!("{canonical_name}-{suffix}"))
                .find(|candidate| !self.bundles.contains_key(candidate))
                .expect("unbounded bundle suffix search")
        };
        self.bundles.insert(name.clone(), desired);
        Ok((name, true))
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

#[cfg(test)]
#[path = "config/tests.rs"]
mod tests;
