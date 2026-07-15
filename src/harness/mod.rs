//! Harness transport engine: `harnesses.json` bundles + a code-owned
//! `(harness, transport)` capability table + profile-mechanism application.
//!
//! This module is self-contained. It touches nothing under `src/identity*`.
//! It supersedes the per-binary sniffing in `session_host::registry` with a
//! static, `(harness, transport)`-keyed driver table and a bundle config
//! surface (`harnesses.json`) that is independent of `agent.json`.

mod codex_profile;
pub mod config;
pub mod driver;
pub mod profile;

use std::path::Path;

pub use config::{HarnessBundle, HarnessesConfig, Transport};
pub use driver::{
    EnvDirective, HarnessDriver, ProfileMechanism, ResumeMechanism, SteerPrimitive, TurnModel,
};
pub use profile::ProfilePlan;

use crate::session::Harness;

/// A fully-resolved bundle: the driver row plus the concrete argv/profile plan.
pub struct ResolvedHarness {
    pub bundle: String,
    pub harness: Harness,
    pub transport: Transport,
    pub driver: &'static HarnessDriver,
    /// Driver argv + bundle args + translated agent profile selector.
    pub base_argv: Vec<String>,
    pub profile: ProfilePlan,
}

/// Resolve an explicit bundle plus the agent's optional harness-specific profile name.
pub fn resolve(
    bundle: &str,
    profile: Option<&str>,
    session_scratch: &Path,
) -> anyhow::Result<ResolvedHarness> {
    let cfg = HarnessesConfig::load()?;
    resolve_with(&cfg, bundle, profile, session_scratch)
}

/// Resolve just the [`Transport`] a bundle drives, without planning its profile
/// or argv. Used by transport selection, which only needs the capability axis.
/// Missing bundle names fail loudly.
pub fn bundle_transport_with(cfg: &HarnessesConfig, bundle: &str) -> anyhow::Result<Transport> {
    cfg.get(bundle)
        .map(|bundle| bundle.transport)
        .ok_or_else(|| anyhow::anyhow!("no harness bundle {bundle:?} in harnesses.json"))
}

/// Resolve just the [`Harness`] a bundle drives (the underlying CLI), without
/// planning its profile.
pub fn bundle_harness_with(cfg: &HarnessesConfig, bundle: &str) -> anyhow::Result<Harness> {
    cfg.get(bundle)
        .map(|bundle| bundle.harness)
        .ok_or_else(|| anyhow::anyhow!("no harness bundle {bundle:?} in harnesses.json"))
}

/// Testable core of [`resolve`] that takes the config explicitly.
pub fn resolve_with(
    cfg: &HarnessesConfig,
    bundle: &str,
    profile: Option<&str>,
    session_scratch: &Path,
) -> anyhow::Result<ResolvedHarness> {
    resolve_with_codex_home(cfg, bundle, profile, session_scratch, None)
}

pub fn native_bundle_with(cfg: &HarnessesConfig, harness: Harness) -> anyhow::Result<String> {
    let preferred = match harness {
        Harness::Codex => [Some(Transport::AppServer), Some(Transport::Pty)],
        Harness::ClaudeCode | Harness::Opencode => [Some(Transport::Pty), None],
        Harness::Grok | Harness::Unknown => [None, None],
    };
    for transport in preferred.into_iter().flatten() {
        let candidates = cfg
            .bundles
            .iter()
            .filter(|(_, bundle)| bundle.harness == harness && bundle.transport == transport)
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        match candidates.as_slice() {
            [name] => return Ok(name.clone()),
            [] => continue,
            _ => anyhow::bail!(
                "multiple {} {} bundles can launch native agents ({}); add an explicit agent harness binding",
                harness.as_str(),
                transport.as_str(),
                candidates.join(", ")
            ),
        }
    }
    anyhow::bail!(
        "no configured hosted bundle can launch native {} agents",
        harness.as_str()
    )
}

pub fn apply_native_agent(
    resolved: &mut ResolvedHarness,
    activation: &crate::agent_catalog::NativeAgentActivation,
    scratch: &Path,
) -> anyhow::Result<()> {
    let plan = match activation {
        crate::agent_catalog::NativeAgentActivation::NativeSelector { name } => {
            profile::plan_profile(resolved.driver.profile, Some(name), scratch, None)?
        }
        crate::agent_catalog::NativeAgentActivation::CodexRoot(agent) => {
            if resolved.harness != Harness::Codex {
                anyhow::bail!("Codex custom-agent activation requires a Codex bundle");
            }
            codex_profile::plan_custom_agent(agent, &codex_profile::source_home()?, scratch)?
        }
    };
    resolved.base_argv.extend(plan.extra_argv.iter().cloned());
    resolved.profile.extend(plan);
    Ok(())
}

fn resolve_with_codex_home(
    cfg: &HarnessesConfig,
    bundle: &str,
    profile: Option<&str>,
    session_scratch: &Path,
    codex_home: Option<&Path>,
) -> anyhow::Result<ResolvedHarness> {
    let configured = cfg
        .get(bundle)
        .ok_or_else(|| anyhow::anyhow!("no harness bundle {bundle:?} in harnesses.json"))?;
    let harness = configured.harness;
    let transport = configured.transport;

    let driver = driver::lookup(harness, transport).ok_or_else(|| {
        anyhow::anyhow!(
            "invalid harness/transport combination: {} x {} (bundle {bundle:?})",
            harness.as_str(),
            transport.as_str()
        )
    })?;

    let plan = profile::plan_profile(driver.profile, profile, session_scratch, codex_home)?;

    let mut base_argv: Vec<String> = driver.base_argv.iter().map(|s| s.to_string()).collect();
    base_argv.extend(configured.args.iter().cloned());
    base_argv.extend(plan.extra_argv.iter().cloned());

    Ok(ResolvedHarness {
        bundle: bundle.to_string(),
        harness,
        transport,
        driver,
        base_argv,
        profile: plan,
    })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
