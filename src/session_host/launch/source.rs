use super::*;
use crate::agent_inventory::AgentSource;
use crate::harness::{HarnessesConfig, Transport};

pub(super) struct ResolvedSource {
    pub(super) transport: TransportImpl,
    pub(super) command: Vec<String>,
    pub(super) harness: crate::session::Harness,
    pub(super) resume: ResumeMechanism,
    pub(super) bundle: String,
    pub(super) native_agent: Option<NativeAgentActivation>,
    pub(super) identity: crate::identity::AgentIdentity,
    pub(super) prepared_launch: crate::session_host::transport::PreparedLaunch,
    pub(super) retired_advertisements: Vec<String>,
}

pub(super) fn resolve_agent_source(
    state: &Arc<DaemonState>,
    selector: &str,
    workspace: &std::path::Path,
    intent: LaunchIntent,
) -> Result<ResolvedSource> {
    let home = crate::config::mosaico_home();
    let mut harnesses = HarnessesConfig::load()?;
    let catalog = state.agent_catalog();
    let installed = state.installed_harnesses();
    let inventory = crate::agent_inventory::AgentInventory::build(
        &home,
        &installed,
        &harnesses,
        &catalog,
        Some(workspace),
    );
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

    let (identity, bundle, profile, native_profile, retired_advertisements) = match selected.source
    {
        AgentSource::Durable {
            bundle,
            profile,
            native_profile,
            ..
        } => {
            let identity = crate::identity::load(&home, &selected.agent_slug)?;
            let native_profile = profile.is_none().then_some(native_profile).flatten();
            (identity, bundle, profile, native_profile, Vec::new())
        }
        AgentSource::DetectedHarness => {
            let bundle = realize_implicit_bundle(&mut harnesses, selected.harness, intent, false)?;
            (
                crate::identity::AgentIdentity::per_session(&selected.agent_slug, &bundle),
                bundle,
                None,
                None,
                Vec::new(),
            )
        }
        AgentSource::DetectedProfile {
            profile: native_profile,
            persist_binding,
        } => {
            let bundle = realize_implicit_bundle(&mut harnesses, selected.harness, intent, true)?;
            let retired = persist_binding.then(|| {
                inventory
                    .profile_choices(&selected.agent_slug)
                    .into_iter()
                    .map(|choice| choice.slug.clone())
                    .collect::<Vec<_>>()
            });
            let identity = if persist_binding {
                state
                    .mutate_agent_config(|| {
                        crate::identity::add_local_agent(
                            &home,
                            &selected.agent_slug,
                            &bundle,
                            None,
                            crate::util::now_secs(),
                        )
                    })?
                    .0
            } else {
                crate::identity::AgentIdentity::per_session(&selected.agent_slug, &bundle)
            };
            (
                identity,
                bundle,
                None,
                Some(native_profile),
                retired.unwrap_or_default(),
            )
        }
    };

    let native_agent = native_profile
        .as_ref()
        .map(|native| native.activation())
        .transpose()?;
    let id = crate::pty::new_endpoint_id(&identity.slug);
    let scratch = home.join("harness-profiles").join(&id);
    let mut resolved =
        crate::harness::resolve_with(&harnesses, &bundle, profile.as_deref(), &scratch)
            .with_context(|| {
                format!("resolving harness bundle {bundle:?} for agent {selector:?}")
            })?;
    if let Some(native_agent) = &native_agent {
        crate::harness::apply_native_agent(&mut resolved, native_agent, &scratch)
            .with_context(|| format!("applying native agent {selector:?}"))?;
    }
    let transport = crate::session_host::transport::select_transport_with(&harnesses, &bundle)?;
    let prepared_launch = transport.prepare_launch(&mut resolved, id)?;
    Ok(ResolvedSource {
        transport,
        command: resolved.base_argv,
        harness: resolved.harness,
        resume: resolved.driver.resume,
        bundle,
        native_agent,
        identity,
        prepared_launch,
        retired_advertisements,
    })
}

fn realize_implicit_bundle(
    harnesses: &mut HarnessesConfig,
    harness: crate::session::Harness,
    intent: LaunchIntent,
    native_profile: bool,
) -> Result<String> {
    let transport = desired_transport(harness, intent, native_profile)?;
    let (bundle, created) = harnesses.resolve_or_create_hosted(harness, transport)?;
    if created {
        harnesses.save_to(&crate::config::mosaico_home().join("harnesses.json"))?;
    }
    Ok(bundle)
}

fn desired_transport(
    harness: crate::session::Harness,
    intent: LaunchIntent,
    native_profile: bool,
) -> Result<Transport> {
    let preferred = match intent {
        LaunchIntent::Interactive => [Some(Transport::Pty), None],
        LaunchIntent::Managed => match harness {
            crate::session::Harness::Codex => [Some(Transport::AppServer), Some(Transport::Pty)],
            crate::session::Harness::ClaudeCode | crate::session::Harness::Opencode => {
                [Some(Transport::Acp), Some(Transport::Pty)]
            }
            crate::session::Harness::Grok => [Some(Transport::Pty), None],
            crate::session::Harness::Unknown => [None, None],
        },
    };
    preferred
        .into_iter()
        .flatten()
        .find(|transport| {
            crate::harness::driver::lookup(harness, *transport).is_some()
                && (!native_profile || crate::harness::supports_native_agent(harness, *transport))
        })
        .with_context(|| {
            format!(
                "{} has no {} hosted transport{}",
                harness.as_str(),
                match intent {
                    LaunchIntent::Interactive => "interactive",
                    LaunchIntent::Managed => "managed",
                },
                if native_profile {
                    " that can activate native profiles"
                } else {
                    ""
                }
            )
        })
}

#[cfg(test)]
#[path = "source/tests.rs"]
mod tests;
