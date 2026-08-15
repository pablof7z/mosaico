//! Harness transport engine: canonical harnesses + a code-owned
//! `(harness, transport)` capability table + profile and preset application.
//!
//! This module is self-contained. It touches nothing under `src/identity*`.
//! It supersedes the per-binary sniffing in `session_host::registry` with a
//! static, `(harness, transport)`-keyed driver table. Agent configuration names
//! the harness; launch intent chooses transport; `presets.json` only adds args.

mod codex_profile;
pub mod driver;
pub mod presets;
pub mod profile;
mod transport;

use std::path::Path;

pub use driver::{
    EnvDirective, HarnessDriver, ProfileMechanism, ResumeMechanism, SteerPrimitive, TurnModel,
};
pub use presets::PresetsConfig;
pub use profile::ProfilePlan;
pub use transport::Transport;

use crate::session::Harness;

/// A fully resolved launch: driver row plus concrete argv/profile plan.
pub struct ResolvedHarness {
    pub harness: Harness,
    pub transport: Transport,
    pub preset: Option<String>,
    pub driver: &'static HarnessDriver,
    /// Driver argv + preset args + translated agent profile selector.
    pub base_argv: Vec<String>,
    pub profile: ProfilePlan,
}

/// Resolve one canonical harness and selected transport plus optional profile and preset.
pub fn resolve(
    harness: Harness,
    transport: Transport,
    profile: Option<&str>,
    preset: Option<&str>,
    session_scratch: &Path,
) -> anyhow::Result<ResolvedHarness> {
    let presets = PresetsConfig::load()?;
    resolve_with(
        &presets,
        harness,
        transport,
        profile,
        preset,
        session_scratch,
    )
}

/// Testable core of [`resolve`] that takes the config explicitly.
pub fn resolve_with(
    presets: &PresetsConfig,
    harness: Harness,
    transport: Transport,
    profile: Option<&str>,
    preset: Option<&str>,
    session_scratch: &Path,
) -> anyhow::Result<ResolvedHarness> {
    resolve_with_codex_home(
        presets,
        harness,
        transport,
        profile,
        preset,
        session_scratch,
        None,
    )
}

pub fn apply_native_agent(
    resolved: &mut ResolvedHarness,
    activation: &crate::agent_catalog::NativeAgentActivation,
    scratch: &Path,
) -> anyhow::Result<()> {
    if matches!(
        (resolved.harness, resolved.transport, activation),
        (
            Harness::ClaudeCode,
            Transport::Acp,
            crate::agent_catalog::NativeAgentActivation::NativeSelector { .. }
        )
    ) {
        return Ok(());
    }
    let plan = match activation {
        crate::agent_catalog::NativeAgentActivation::NativeSelector { name } => {
            profile::plan_profile(resolved.driver.profile, Some(name), scratch, None)?
        }
        crate::agent_catalog::NativeAgentActivation::CodexRoot(agent) => {
            if resolved.harness != Harness::Codex {
                anyhow::bail!("Codex custom-agent activation requires the Codex harness");
            }
            if resolved.transport == Transport::AppServer {
                ProfilePlan::default()
            } else {
                codex_profile::plan_custom_agent(agent, &codex_profile::source_home()?, scratch)?
            }
        }
    };
    resolved
        .base_argv
        .splice(1..1, plan.global_argv.iter().cloned());
    resolved.base_argv.extend(plan.extra_argv.iter().cloned());
    resolved.profile.extend(plan);
    Ok(())
}

pub fn supports_native_agent(harness: Harness, transport: Transport) -> bool {
    matches!(
        (harness, transport),
        (Harness::ClaudeCode, Transport::Pty | Transport::Acp)
            | (Harness::Codex, Transport::Pty | Transport::AppServer)
            | (Harness::Opencode, Transport::Pty)
            | (Harness::Hermes, Transport::Pty | Transport::Acp)
            | (Harness::Kimi, Transport::Pty)
    )
}

fn resolve_with_codex_home(
    presets: &PresetsConfig,
    harness: Harness,
    transport: Transport,
    profile: Option<&str>,
    preset: Option<&str>,
    session_scratch: &Path,
    codex_home: Option<&Path>,
) -> anyhow::Result<ResolvedHarness> {
    let driver = driver::lookup(harness, transport).ok_or_else(|| {
        anyhow::anyhow!(
            "unsupported harness/transport combination: {} x {}",
            harness.as_str(),
            transport.as_str()
        )
    })?;

    let plan = profile::plan_profile(driver.profile, profile, session_scratch, codex_home)?;

    let mut base_argv: Vec<String> = driver.base_argv.iter().map(|s| s.to_string()).collect();
    base_argv.splice(1..1, plan.global_argv.iter().cloned());
    base_argv.extend(presets.args(preset, harness, transport)?);
    base_argv.extend(plan.extra_argv.iter().cloned());

    Ok(ResolvedHarness {
        harness,
        transport,
        preset: preset.map(str::to_string),
        driver,
        base_argv,
        profile: plan,
    })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
