use std::collections::HashSet;
use std::fmt;
use std::io::IsTerminal;

use anyhow::{bail, Context as _, Result};

use crate::harness::{driver, HarnessesConfig, Transport};
use crate::session::Harness;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RpcLaunchChoice {
    PtyBundle(String),
    Headless,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PickerOption {
    choice: RpcLaunchChoice,
    label: String,
}

impl fmt::Display for PickerOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

/// Return the configured RPC bundle for an agent. Launch is a user-facing
/// boundary, so configuration errors fail loudly here instead of silently
/// changing the requested transport.
pub(super) fn configured_rpc_bundle(agent: &str) -> Result<Option<String>> {
    let Some(bundle) = crate::identity::agent_harness_bundle(&crate::config::edge_home(), agent)
    else {
        return Ok(None);
    };
    let cfg = HarnessesConfig::load()?;
    let transport = crate::harness::bundle_transport_with(&cfg, &bundle)
        .with_context(|| format!("resolving configured harness bundle {bundle:?}"))?;
    Ok(matches!(transport, Transport::Acp | Transport::AppServer).then_some(bundle))
}

/// Validate an explicit `--harness` bundle. This flag deliberately names only
/// attachable PTY bundles; headless launch is the final interactive choice for
/// an RPC-configured agent.
pub(super) fn validate_explicit_pty_bundle(bundle: &str) -> Result<String> {
    let cfg = HarnessesConfig::load()?;
    validate_explicit_pty_bundle_with(&cfg, bundle)
}

fn validate_explicit_pty_bundle_with(cfg: &HarnessesConfig, bundle: &str) -> Result<String> {
    let transport = crate::harness::bundle_transport_with(cfg, bundle)
        .with_context(|| format!("resolving --harness {bundle:?}"))?;
    if transport != Transport::Pty {
        bail!(
            "--harness {bundle:?} uses the {} transport, which cannot attach to a terminal; \
             choose a PTY bundle",
            transport.as_str()
        );
    }
    let harness = crate::harness::bundle_harness_with(cfg, bundle)?;
    if driver::lookup(harness, Transport::Pty).is_none() {
        bail!(
            "--harness {bundle:?} has no PTY driver for {}",
            harness.as_str()
        );
    }
    Ok(bundle.to_string())
}

/// Interactive choice shown only when the agent's configured bundle is RPC.
/// Non-TTY callers necessarily choose headless because no attachment is
/// possible and no prompt can be rendered.
pub(super) fn choose_rpc_launch(configured_bundle: &str) -> Result<RpcLaunchChoice> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Ok(RpcLaunchChoice::Headless);
    }
    let cfg = HarnessesConfig::load()?;
    let options = picker_options(&cfg, configured_bundle)?;
    inquire::Select::new(
        &format!("{configured_bundle} is headless. How should this agent launch?"),
        options,
    )
    .with_help_message("↑/↓ choose · enter launch")
    .prompt()
    .map(|option| option.choice)
    .map_err(|error| anyhow::anyhow!("launch selection failed: {error}"))
}

fn picker_options(cfg: &HarnessesConfig, configured_bundle: &str) -> Result<Vec<PickerOption>> {
    let preferred = crate::harness::bundle_harness_with(cfg, configured_bundle)
        .with_context(|| format!("resolving configured harness bundle {configured_bundle:?}"))?;
    let mut bundles = configured_pty_bundles(cfg);
    add_builtin_pty_bundles(cfg, &mut bundles);
    bundles.sort_by(|(name_a, harness_a), (name_b, harness_b)| {
        (*harness_a != preferred)
            .cmp(&(*harness_b != preferred))
            .then_with(|| harness_label(*harness_a).cmp(harness_label(*harness_b)))
            .then_with(|| name_a.cmp(name_b))
    });

    let mut options = bundles
        .into_iter()
        .map(|(bundle, harness)| PickerOption {
            label: format!("{bundle} — {} (PTY, attach)", harness_label(harness)),
            choice: RpcLaunchChoice::PtyBundle(bundle),
        })
        .collect::<Vec<_>>();
    options.push(PickerOption {
        choice: RpcLaunchChoice::Headless,
        label: format!("Launch {configured_bundle} headless (no terminal attachment)"),
    });
    Ok(options)
}

fn configured_pty_bundles(cfg: &HarnessesConfig) -> Vec<(String, Harness)> {
    cfg.bundles
        .iter()
        .filter(|(_, bundle)| {
            bundle.transport == Transport::Pty
                && driver::lookup(bundle.harness, Transport::Pty).is_some()
        })
        .map(|(name, bundle)| (name.clone(), bundle.harness))
        .collect()
}

fn add_builtin_pty_bundles(cfg: &HarnessesConfig, bundles: &mut Vec<(String, Harness)>) {
    let mut seen = bundles
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<HashSet<_>>();
    for row in driver::all()
        .iter()
        .filter(|row| row.transport == Transport::Pty)
    {
        let name = builtin_bundle_name(row.harness).to_string();
        // A configured bundle owns its name. If it shadows a built-in with an
        // RPC transport, do not mislabel that name as PTY.
        if cfg.get(&name).is_none() && seen.insert(name.clone()) {
            bundles.push((name, row.harness));
        }
    }
}

fn builtin_bundle_name(harness: Harness) -> &'static str {
    match harness {
        Harness::ClaudeCode => "claude",
        Harness::Codex => "codex",
        Harness::Opencode => "opencode",
        Harness::Grok => "grok",
        Harness::Unknown => "unknown",
    }
}

fn harness_label(harness: Harness) -> &'static str {
    match harness {
        Harness::ClaudeCode => "Claude Code",
        Harness::Codex => "Codex",
        Harness::Opencode => "OpenCode",
        Harness::Grok => "Grok",
        Harness::Unknown => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(json: &str) -> HarnessesConfig {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn picker_puts_same_family_first_and_headless_last() {
        let cfg = config(
            r#"{
                "claude-acp": {"harness":"claude", "transport":"acp"},
                "codex-yolo": {"harness":"codex", "transport":"pty"}
            }"#,
        );
        let options = picker_options(&cfg, "claude-acp").unwrap();

        assert_eq!(
            options.first().unwrap().choice,
            RpcLaunchChoice::PtyBundle("claude".into())
        );
        assert_eq!(options.last().unwrap().choice, RpcLaunchChoice::Headless);
        assert!(options
            .iter()
            .any(|option| { option.choice == RpcLaunchChoice::PtyBundle("codex-yolo".into()) }));
    }

    #[test]
    fn configured_name_shadows_builtin_transport() {
        let cfg = config(
            r#"{
                "claude": {"harness":"claude", "transport":"acp"}
            }"#,
        );
        let options = picker_options(&cfg, "claude").unwrap();
        assert!(!options
            .iter()
            .any(|option| { option.choice == RpcLaunchChoice::PtyBundle("claude".into()) }));
    }

    #[test]
    fn explicit_harness_rejects_non_pty_bundle() {
        let cfg = config(
            r#"{
                "claude-acp": {"harness":"claude", "transport":"acp"}
            }"#,
        );
        let error = validate_explicit_pty_bundle_with(&cfg, "claude-acp").unwrap_err();
        assert!(error.to_string().contains("cannot attach to a terminal"));
        assert_eq!(
            validate_explicit_pty_bundle_with(&cfg, "codex").unwrap(),
            "codex"
        );
    }
}
