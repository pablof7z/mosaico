//! Harness transport engine: `harnesses.json` bundles + a code-owned
//! `(harness, transport)` capability table + profile-mechanism application.
//!
//! This module is self-contained. It touches nothing under `src/identity*`.
//! It supersedes the per-binary sniffing in `session_host::registry` with a
//! static, `(harness, transport)`-keyed driver table and a bundle config
//! surface (`harnesses.json`) that is independent of `agent.json`.

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
    /// `base_argv` + profile `extra_argv`, before the agent's own user flags.
    pub base_argv: Vec<String>,
    pub profile: ProfilePlan,
}

/// Resolve a bundle name against `harnesses.json`, falling back to a built-in
/// default (`bundle == harness slug`, transport = Pty) so existing
/// `claude`/`codex`/`opencode`/`grok` spawns keep working with zero config.
///
/// `session_scratch` is a per-session directory into which any settings file is
/// materialized (never the user's repo).
pub fn resolve(bundle: &str, session_scratch: &Path) -> anyhow::Result<ResolvedHarness> {
    let cfg = HarnessesConfig::load()?;
    resolve_with(&cfg, bundle, session_scratch)
}

/// Resolve just the [`Transport`] a bundle drives, without planning its profile
/// or argv. Used by transport selection, which only needs the capability axis.
/// Fails loud on the same conditions as [`resolve`] (unknown bundle that is not
/// a built-in harness slug).
pub fn bundle_transport_with(cfg: &HarnessesConfig, bundle: &str) -> anyhow::Result<Transport> {
    match cfg.get(bundle) {
        Some(b) => Ok(b.transport),
        None => {
            let harness = Harness::from_str(bundle);
            if harness == Harness::Unknown {
                anyhow::bail!(
                    "no harness bundle {bundle:?} in harnesses.json and it is not a built-in \
                     harness slug (claude|codex|opencode|grok)"
                );
            }
            // Built-in default: bundle name IS a harness slug -> interactive PTY.
            Ok(Transport::Pty)
        }
    }
}

/// Resolve just the [`Harness`] a bundle drives (the underlying CLI), without
/// planning its profile. Mirrors [`bundle_transport_with`]'s fallback rules.
pub fn bundle_harness_with(cfg: &HarnessesConfig, bundle: &str) -> anyhow::Result<Harness> {
    match cfg.get(bundle) {
        Some(b) => Ok(b.harness),
        None => {
            let harness = Harness::from_str(bundle);
            if harness == Harness::Unknown {
                anyhow::bail!(
                    "no harness bundle {bundle:?} in harnesses.json and it is not a built-in \
                     harness slug (claude|codex|opencode|grok)"
                );
            }
            Ok(harness)
        }
    }
}

/// Testable core of [`resolve`] that takes the config explicitly.
pub fn resolve_with(
    cfg: &HarnessesConfig,
    bundle: &str,
    session_scratch: &Path,
) -> anyhow::Result<ResolvedHarness> {
    let (harness, transport, profile_val) = match cfg.get(bundle) {
        Some(b) => (b.harness, b.transport, b.profile.clone()),
        None => {
            // Built-in default: the bundle name IS a harness slug, driven over
            // the interactive PTY transport (byte-identical to today's spawn).
            let harness = Harness::from_str(bundle);
            if harness == Harness::Unknown {
                anyhow::bail!(
                    "no harness bundle {bundle:?} in harnesses.json and it is not a built-in \
                     harness slug (claude|codex|opencode|grok)"
                );
            }
            (harness, Transport::Pty, None)
        }
    };

    let driver = driver::lookup(harness, transport).ok_or_else(|| {
        anyhow::anyhow!(
            "invalid harness/transport combination: {} x {} (bundle {bundle:?})",
            harness.as_str(),
            transport.as_str()
        )
    })?;

    let plan = profile::plan_profile(
        harness,
        driver.profile,
        profile_val.as_ref(),
        session_scratch,
    )?;

    let mut base_argv: Vec<String> = driver.base_argv.iter().map(|s| s.to_string()).collect();
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
