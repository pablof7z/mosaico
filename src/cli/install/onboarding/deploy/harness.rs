//! Resolve an onboarding harness selection to an RPC (ACP / app-server) bundle
//! for the relay-assist modal. Terminal-only harnesses (Grok) are excluded.

use anyhow::Result;
use std::path::PathBuf;

use crate::harness::config::{HarnessesConfig, Transport};
use crate::harness::ResolvedHarness;
use crate::session::Harness as SessionHarness;

/// The RPC transport an onboarding harness id can be driven over, or `None` for
/// terminal-only harnesses that cannot join the structured modal.
pub(in crate::cli::install::onboarding) fn rpc_transport(
    id: &str,
) -> Option<(SessionHarness, Transport)> {
    match id {
        "claude-code" => Some((SessionHarness::ClaudeCode, Transport::Acp)),
        "codex" => Some((SessionHarness::Codex, Transport::AppServer)),
        "opencode" => Some((SessionHarness::Opencode, Transport::Acp)),
        "goose" => Some((SessionHarness::Goose, Transport::Acp)),
        "hermes" => Some((SessionHarness::Hermes, Transport::Acp)),
        // Grok is PTY-only — no structured transport.
        _ => None,
    }
}

/// Whether an onboarding harness id can drive the structured assist modal.
pub(in crate::cli::install::onboarding) fn can_assist(id: &str) -> bool {
    rpc_transport(id).is_some()
}

/// A resolved harness plus the scratch working directory for the assist session.
pub(in crate::cli::install::onboarding) struct DeployTarget {
    pub resolved: ResolvedHarness,
    pub cwd: PathBuf,
}

/// Resolve `id` to a spawnable RPC bundle, seeding a zero-argument bundle in
/// memory when the operator has none configured (never persisted).
pub(in crate::cli::install::onboarding) fn resolve(id: &str) -> Result<DeployTarget> {
    let (harness, transport) = rpc_transport(id)
        .ok_or_else(|| anyhow::anyhow!("{id} is terminal-only and cannot host the assist modal"))?;

    let mut cfg = HarnessesConfig::load()?;
    let (bundle, _created) = cfg.resolve_or_create_hosted(harness, transport)?;

    let scratch = crate::config::mosaico_home()
        .join("harness-profiles")
        .join(&bundle);
    let cwd = crate::config::mosaico_home().join("relay-assist");
    std::fs::create_dir_all(&cwd)?;

    let resolved = crate::harness::resolve_with(&cfg, &bundle, None, &scratch)?;
    Ok(DeployTarget { resolved, cwd })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_rpc_harnesses_and_excludes_grok() {
        assert!(can_assist("claude-code"));
        assert!(can_assist("codex"));
        assert!(can_assist("opencode"));
        assert!(can_assist("goose"));
        assert!(can_assist("hermes"));
        assert!(!can_assist("grok"));
    }

    #[test]
    fn transport_selection_is_correct() {
        assert!(matches!(
            rpc_transport("codex"),
            Some((SessionHarness::Codex, Transport::AppServer))
        ));
        assert!(matches!(
            rpc_transport("claude-code"),
            Some((SessionHarness::ClaudeCode, Transport::Acp))
        ));
    }
}
