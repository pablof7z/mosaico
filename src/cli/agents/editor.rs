use super::data::{harness_name, AgentKind, AgentRow};
use crate::harness::{HarnessBundle, HarnessesConfig, Transport};
use crate::session::Harness;
use anyhow::{bail, Context, Result};
use dialoguer::{theme::ColorfulTheme, Select};
use std::io::IsTerminal as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OperationMode {
    Acp,
    Pty,
}

impl OperationMode {
    fn label(self) -> &'static str {
        match self {
            Self::Acp => "ACP — optimized for headless mode",
            Self::Pty => "PTY — optimized for direct user interaction",
        }
    }
}

pub(super) async fn edit(row: &AgentRow) -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        bail!("agent editing is interactive — run it in a terminal");
    }
    let theme = ColorfulTheme::default();
    let harness = select_harness(row, &theme)?;
    let options = operation_modes(harness, row.native_profile.is_some());
    if options.is_empty() {
        bail!(
            "{} has no hosted transport that can activate this agent",
            harness_name(harness)
        );
    }
    let default_mode = row
        .transport
        .and_then(|transport| {
            options
                .iter()
                .position(|mode| mode_transport(harness, *mode) == transport)
        })
        .unwrap_or(0);
    let mode = if options.len() == 1 {
        options[0]
    } else {
        let labels = options.iter().map(|mode| mode.label()).collect::<Vec<_>>();
        options[Select::with_theme(&theme)
            .with_prompt("How should this agent run?")
            .items(&labels)
            .default(default_mode)
            .interact()?]
    };
    let transport = mode_transport(harness, mode);
    let bundle = select_or_create_bundle(harness, transport, row.bundle.as_deref(), &theme)?;
    let per_session_key = select_key_mode(row.per_session_key.unwrap_or(true), &theme)?;
    let profile = profile_for_save(row);
    let saved =
        super::save_agent_config(&row.slug, &bundle, profile, Some(per_session_key)).await?;
    println!(
        "{} {} · {bundle} · {}",
        if saved.created { "Created" } else { "Updated" },
        row.slug,
        mode.label()
    );
    Ok(())
}

fn profile_for_save(row: &AgentRow) -> Option<String> {
    (row.kind != AgentKind::NativeProfile)
        .then(|| row.profile.clone())
        .flatten()
}

fn select_harness(row: &AgentRow, theme: &ColorfulTheme) -> Result<Harness> {
    if row.harness != Harness::Unknown {
        return Ok(row.harness);
    }
    let available = crate::config::detect_available_harnesses()?
        .into_iter()
        .filter(|harness| *harness != Harness::Unknown)
        .collect::<Vec<_>>();
    if available.is_empty() {
        bail!("the configured bundle is missing and no local harness is available");
    }
    let labels = available
        .iter()
        .map(|harness| harness_name(*harness))
        .collect::<Vec<_>>();
    Ok(available[Select::with_theme(theme)
        .with_prompt("Select harness")
        .items(&labels)
        .default(0)
        .interact()?])
}

fn operation_modes(harness: Harness, native_profile: bool) -> Vec<OperationMode> {
    [OperationMode::Acp, OperationMode::Pty]
        .into_iter()
        .filter(|mode| {
            let transport = mode_transport(harness, *mode);
            crate::harness::driver::lookup(harness, transport).is_some()
                && (!native_profile || crate::harness::supports_native_agent(harness, transport))
        })
        .collect()
}

fn mode_transport(harness: Harness, mode: OperationMode) -> Transport {
    match (harness, mode) {
        (Harness::Codex, OperationMode::Acp) => Transport::AppServer,
        (_, OperationMode::Acp) => Transport::Acp,
        (_, OperationMode::Pty) => Transport::Pty,
    }
}

fn select_or_create_bundle(
    harness: Harness,
    transport: Transport,
    current: Option<&str>,
    theme: &ColorfulTheme,
) -> Result<String> {
    let path = crate::config::mosaico_home().join("harnesses.json");
    let mut config = HarnessesConfig::load_from(&path)?;
    let compatible = compatible_bundles(&config, harness, transport);
    if !compatible.is_empty() {
        let default = current
            .and_then(|current| compatible.iter().position(|name| name == current))
            .unwrap_or(0);
        if compatible.len() == 1 {
            return Ok(compatible[0].clone());
        }
        return Ok(compatible[Select::with_theme(theme)
            .with_prompt("Select harness configuration")
            .items(&compatible)
            .default(default)
            .interact()?]
        .clone());
    }

    let (name, created) = config.ensure_bundle(
        &format!("{}-{}", harness.agent_slug(), transport.as_str()),
        HarnessBundle {
            harness,
            transport,
            args: Vec::new(),
        },
    )?;
    if created {
        config.save_to(&path).with_context(|| {
            format!("saving automatically-created harness configuration {name:?}")
        })?;
        println!("Created harness configuration {name}");
    }
    Ok(name)
}

fn compatible_bundles(
    config: &HarnessesConfig,
    harness: Harness,
    transport: Transport,
) -> Vec<String> {
    config
        .bundles
        .iter()
        .filter(|(_, bundle)| {
            bundle.harness == harness
                && bundle.transport == transport
                && crate::harness::driver::lookup(harness, transport).is_some()
        })
        .map(|(name, _)| name.clone())
        .collect()
}

fn select_key_mode(current_per_session: bool, theme: &ColorfulTheme) -> Result<bool> {
    let options = [
        "Per-session key — a fresh identity for every session",
        "Persistent key — reuse one identity across sessions",
    ];
    Ok(Select::with_theme(theme)
        .with_prompt("Agent identity")
        .items(&options)
        .default(usize::from(!current_per_session))
        .interact()?
        == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conceptual_acp_maps_to_each_harness_native_rpc_transport() {
        assert_eq!(
            mode_transport(Harness::ClaudeCode, OperationMode::Acp),
            Transport::Acp
        );
        assert_eq!(
            mode_transport(Harness::Codex, OperationMode::Acp),
            Transport::AppServer
        );
    }

    #[test]
    fn native_profiles_only_offer_transports_that_activate_them() {
        assert_eq!(
            operation_modes(Harness::ClaudeCode, true),
            [OperationMode::Acp, OperationMode::Pty]
        );
        assert_eq!(
            operation_modes(Harness::Opencode, true),
            [OperationMode::Pty]
        );
    }

    #[test]
    fn compatible_bundle_filter_never_crosses_harnesses() {
        let config: HarnessesConfig = serde_json::from_str(
            r#"{"claude-acp":{"harness":"claude","transport":"acp"},"codex-pty":{"harness":"codex","transport":"pty"}}"#,
        )
        .unwrap();
        assert_eq!(
            compatible_bundles(&config, Harness::ClaudeCode, Transport::Acp),
            ["claude-acp"]
        );
    }

    #[test]
    fn editing_a_configured_agent_preserves_its_explicit_profile() {
        let row = AgentRow {
            slug: "reviewer".into(),
            agent_slug: "reviewer".into(),
            description: "Reviews".into(),
            harness: Harness::ClaudeCode,
            bundle: Some("claude-pty".into()),
            transport: Some(Transport::Pty),
            profile: Some("specialist".into()),
            per_session_key: Some(true),
            kind: AgentKind::Configured,
            native_profile: Some(crate::agent_catalog::NativeAgentProfile {
                slug: "reviewer".into(),
                use_criteria: "Reviews".into(),
                harness: Harness::ClaudeCode,
                scope: crate::agent_catalog::AgentScope::Global,
                path: "/tmp/reviewer.md".into(),
                modified_at: 1,
            }),
        };

        assert_eq!(profile_for_save(&row).as_deref(), Some("specialist"));
    }
}
