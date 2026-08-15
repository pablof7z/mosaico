use super::*;
use crate::agent_catalog::NativeAgentActivation;
use crate::agent_inventory::AgentSource;
use crate::harness::{PresetsConfig, ResumeMechanism, Transport};
use crate::session_host::transport::TransportImpl;

pub(super) struct ResolvedSource {
    pub(super) transport: TransportImpl,
    pub(super) command: Vec<String>,
    pub(super) harness: crate::session::Harness,
    pub(super) resume: ResumeMechanism,
    pub(super) preset: Option<String>,
    pub(super) native_agent: Option<NativeAgentActivation>,
    pub(super) identity: crate::identity::AgentIdentity,
    pub(super) prepared_launch: crate::session_host::transport::PreparedLaunch,
}

pub(super) fn resolve_agent_source(
    state: &Arc<DaemonState>,
    selector: &str,
    workspace: &std::path::Path,
    intent: LaunchIntent,
) -> Result<ResolvedSource> {
    let home = crate::config::mosaico_home();
    let catalog = state.agent_catalog();
    let installed = state.installed_harnesses();
    let inventory =
        crate::agent_inventory::AgentInventory::build(&home, &installed, &catalog, Some(workspace));
    let selected = inventory.find(selector).cloned().with_context(|| {
        let choices = inventory.profile_choices(selector);
        if choices.is_empty() {
            format!("no available agent or harness named {selector:?}")
        } else {
            format!(
                "agent {selector:?} is available from multiple harnesses; choose {}",
                choices
                    .iter()
                    .map(|choice| choice.slug.as_str())
                    .collect::<Vec<_>>()
                    .join(" or ")
            )
        }
    })?;

    let (identity, profile, preset, native_profile) = match selected.source {
        AgentSource::Durable {
            profile,
            preset,
            native_profile,
            ..
        } => {
            let identity = crate::identity::load(&home, &selected.agent_slug)?;
            let native_profile = profile.is_none().then_some(native_profile).flatten();
            (identity, profile, preset, native_profile)
        }
        AgentSource::DetectedHarness => (
            crate::identity::AgentIdentity::per_session(
                &selected.agent_slug,
                selected.harness.as_str(),
            ),
            None,
            None,
            None,
        ),
        AgentSource::DetectedProfile {
            profile: native_profile,
            persist_binding,
        } => {
            let identity = if persist_binding {
                state
                    .mutate_agent_config(|| {
                        crate::identity::add_local_agent(
                            &home,
                            &selected.agent_slug,
                            selected.harness.as_str(),
                            None,
                            None,
                            crate::util::now_secs(),
                        )
                    })?
                    .0
            } else {
                crate::identity::AgentIdentity::per_session(
                    &selected.agent_slug,
                    selected.harness.as_str(),
                )
            };
            (identity, None, None, Some(native_profile))
        }
    };

    let transport = desired_transport(
        selected.harness,
        intent,
        profile.is_some(),
        native_profile.is_some(),
    )?;

    finish_source(
        &home,
        selected.harness,
        transport,
        profile.as_deref(),
        preset.as_deref(),
        native_profile.as_ref(),
        identity,
        selector,
    )
}

/// Resolve only the harness-owned resume policy. A resumed
/// session's logical agent and signer come from its persisted session row, not
/// from the current agent inventory, so stale or removed profile bindings
/// cannot silently change its identity.
pub(super) fn resolve_harness_source(
    harness: crate::session::Harness,
    slug: &str,
    admitted_transport: Option<&str>,
    intent: LaunchIntent,
) -> Result<ResolvedSource> {
    let home = crate::config::mosaico_home();
    let transport = match intent {
        LaunchIntent::Interactive => desired_transport(harness, intent, false, false)?,
        LaunchIntent::Managed => admitted_transport
            .and_then(transport_from_str)
            .filter(|transport| crate::harness::driver::lookup(harness, *transport).is_some())
            .map(Ok)
            .unwrap_or_else(|| desired_transport(harness, intent, false, false))?,
    };
    let preset = if crate::identity::is_configured(&home, slug) {
        let config = crate::identity::agent_launch_config(&home, slug)?;
        anyhow::ensure!(
            config.harness == harness.as_str(),
            "agent {slug:?} is configured for harness {:?}, not {}",
            config.harness,
            harness.as_str()
        );
        config.preset
    } else {
        None
    };
    let identity = crate::identity::AgentIdentity::per_session(slug, harness.as_str());
    finish_source(
        &home,
        harness,
        transport,
        None,
        preset.as_deref(),
        None,
        identity,
        slug,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish_source(
    home: &std::path::Path,
    harness: crate::session::Harness,
    selected_transport: Transport,
    profile: Option<&str>,
    preset: Option<&str>,
    native_profile: Option<&crate::agent_catalog::NativeAgentProfile>,
    identity: crate::identity::AgentIdentity,
    selector: &str,
) -> Result<ResolvedSource> {
    let native_agent = native_profile
        .map(|native| native.activation())
        .transpose()?;
    let id = crate::pty::new_endpoint_id(&identity.slug);
    let scratch = home.join("harness-profiles").join(&id);
    let presets = PresetsConfig::load()?;
    let mut resolved = crate::harness::resolve_with(
        &presets,
        harness,
        selected_transport,
        profile,
        preset,
        &scratch,
    )
    .with_context(|| {
        format!(
            "resolving {} launch for agent {selector:?}",
            harness.as_str()
        )
    })?;
    if let Some(native_agent) = &native_agent {
        crate::harness::apply_native_agent(&mut resolved, native_agent, &scratch)
            .with_context(|| format!("applying native agent {selector:?}"))?;
    }
    let transport = crate::session_host::transport::select_transport(selected_transport);
    let prepared_launch = transport.prepare_launch(&mut resolved, id)?;
    Ok(ResolvedSource {
        transport,
        command: resolved.base_argv,
        harness: resolved.harness,
        resume: resolved.driver.resume,
        preset: preset.map(str::to_string),
        native_agent,
        identity,
        prepared_launch,
    })
}

fn desired_transport(
    harness: crate::session::Harness,
    intent: LaunchIntent,
    profile: bool,
    native_profile: bool,
) -> Result<Transport> {
    let preferred = match intent {
        LaunchIntent::Interactive => [Some(Transport::Pty), None],
        LaunchIntent::Managed => match harness {
            crate::session::Harness::Codex => [Some(Transport::AppServer), Some(Transport::Pty)],
            crate::session::Harness::ClaudeCode
            | crate::session::Harness::Opencode
            | crate::session::Harness::Hermes
            | crate::session::Harness::Kimi => [Some(Transport::Acp), Some(Transport::Pty)],
            crate::session::Harness::Grok => [Some(Transport::Pty), None],
            crate::session::Harness::Goose => [Some(Transport::Acp), None],
            crate::session::Harness::Pi => [Some(Transport::PiRpc), Some(Transport::Pty)],
            crate::session::Harness::Unknown => [None, None],
        },
    };
    preferred
        .into_iter()
        .flatten()
        .find(|transport| {
            crate::harness::driver::lookup(harness, *transport).is_some_and(|driver| {
                (!profile || driver.profile != crate::harness::ProfileMechanism::Unsupported)
                    && (!native_profile
                        || crate::harness::supports_native_agent(harness, *transport))
            })
        })
        .with_context(|| {
            format!(
                "{} has no {} hosted transport{}",
                harness.as_str(),
                match intent {
                    LaunchIntent::Interactive => "interactive",
                    LaunchIntent::Managed => "managed",
                },
                if native_profile || profile {
                    " that can activate the selected profile"
                } else {
                    ""
                }
            )
        })
}

fn transport_from_str(value: &str) -> Option<Transport> {
    match value {
        "pty" => Some(Transport::Pty),
        "acp" => Some(Transport::Acp),
        "app-server" => Some(Transport::AppServer),
        "pi-rpc" => Some(Transport::PiRpc),
        _ => None,
    }
}

#[cfg(test)]
#[path = "source/tests.rs"]
mod tests;
