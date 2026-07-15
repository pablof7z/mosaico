use super::args::LaunchRequest;
use anyhow::{Context as _, Result};

// ── launch ───────────────────────────────────────────────────────────────────

/// Attach or resume a named session, or launch a fresh harness if it is unknown.
///
/// A fresh launch spawns an independent portable-pty supervisor, starts the
/// selected harness inside it, then attaches the current terminal.
pub(super) async fn launch(request: LaunchRequest) -> Result<()> {
    if super::existing::launch_if_known(&request).await? {
        return Ok(());
    }
    let LaunchRequest {
        agent,
        root,
        channel,
        session_name,
        command_name,
        harness,
        headless,
        override_command,
        extra_args,
        prompt,
    } = request;
    let root = match root {
        Some(p) => p,
        None => crate::workspace::resolve_or_bail(&std::env::current_dir().unwrap_or_default())?,
    };

    if headless {
        super::harness_picker::configured_rpc_bundle(&agent)?.with_context(|| {
            format!(
                "--headless requires agent {agent:?} to select an ACP/app-server harness bundle"
            )
        })?;
        let channel = resolve_launch_channel(&root, &agent, channel).await?;
        return launch_acp_headless(agent, root, channel, session_name, extra_args, prompt).await;
    }

    let pty_source = if let Some(bundle) = harness {
        super::pty_launch::CommandSource::Bundle(
            super::harness_picker::validate_explicit_pty_bundle(&bundle)?,
        )
    } else if !override_command.is_empty() {
        super::pty_launch::CommandSource::Command(override_command)
    } else if command_name.is_some() {
        super::pty_launch::CommandSource::Command(super::launch_command::resolve_launch_command(
            &agent,
            command_name.as_deref(),
            &extra_args,
        )?)
    } else if let Some(bundle) = super::harness_picker::configured_rpc_bundle(&agent)? {
        match super::harness_picker::choose_rpc_launch(&bundle)? {
            super::harness_picker::RpcLaunchChoice::PtyBundle(bundle) => {
                super::pty_launch::CommandSource::Bundle(bundle)
            }
            super::harness_picker::RpcLaunchChoice::Headless => {
                let channel = resolve_launch_channel(&root, &agent, channel).await?;
                return launch_acp_headless(agent, root, channel, session_name, extra_args, prompt)
                    .await;
            }
        }
    } else {
        super::pty_launch::CommandSource::Command(super::launch_command::resolve_launch_command(
            &agent,
            None,
            &extra_args,
        )?)
    };
    let channel = resolve_launch_channel(&root, &agent, channel).await?;
    super::pty_launch::launch(super::pty_launch::PtyLaunchRequest {
        agent,
        root,
        channel,
        session_name,
        source: pty_source,
        extra_args,
        prompt,
    })
    .await
}

/// Resolve the launch channel shared by the PTY and ACP paths. `--channel ""`
/// opens the interactive picker (TTY required); a bare launch defaults to the
/// workspace channel; a name is resolved to its opaque `channel_h` (created if
/// absent) BEFORE spawning, so MOSAICO_CHANNEL and provisioning see one id.
async fn resolve_launch_channel(
    root: &str,
    agent: &str,
    channel: Option<String>,
) -> Result<Option<String>> {
    let want_picker = matches!(channel, Some(ref s) if s.is_empty());
    if want_picker {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            anyhow::bail!(
                "channel selection needs a TTY to open the interactive picker; \
                 pass --channel <name> to scope into a specific channel non-interactively"
            );
        }
        return Ok(Some(pick_channel(root, agent).await?));
    }
    match channel {
        None => Ok(Some(root.to_string())),
        Some(name) if !name.is_empty() => {
            let v = super::super::daemon_call_async(
                "channel_resolve",
                channel_resolve_params(root, &name, agent),
            )
            .await?;
            Ok(Some(
                v["channel_h"]
                    .as_str()
                    .context("channel_resolve did not return channel_h")?
                    .to_string(),
            ))
        }
        other => Ok(other),
    }
}

fn channel_resolve_params(root: &str, name: &str, agent: &str) -> serde_json::Value {
    serde_json::json!({
        "channel": root,
        "name": name,
        "agent": agent,
        "create_if_absent": true,
    })
}

/// Launch a headless ACP/app-server agent through the daemon. The daemon opens
/// and registers the RPC child (so the doorbell delivery path can reach it),
/// synthesizes the launch argv from the harness bundle, appends `extra_args`, and
/// — if an opening prompt was given — opens the session on it. There is no TTY to
/// attach; the agent thereafter responds to channel mentions.
async fn launch_acp_headless(
    agent: String,
    root: String,
    channel: Option<String>,
    session_name: Option<String>,
    extra_args: Vec<String>,
    prompt: Option<String>,
) -> Result<()> {
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string());
    // Admission (the durable-agent reservation) is handled daemon-side by
    // `spawn_agent`, so no client-side preflight reservation is taken here.
    let mut params = serde_json::json!({
        "agent": agent,
        "root": root,
        "launch": { "kind": "configured" },
        "args": extra_args,
        "channel": channel,
        "session_name": session_name,
        "cwd": cwd,
    });
    if let Some(prompt) = prompt.filter(|s| !s.is_empty()) {
        params["prompt"] = serde_json::Value::String(prompt);
    }
    let v = super::super::daemon_call_async("pty_spawn", params)
        .await
        .with_context(|| format!("headless ACP launch of agent {agent:?} failed"))?;
    let session = v["pty_id"].as_str().unwrap_or_default();
    eprintln!("[mosaico acp] session: {session}");
    eprintln!(
        "[mosaico acp] headless agent launched; it responds to channel mentions (no PTY to attach)"
    );
    Ok(())
}

/// Fetch all rooms under `root` and present an interactive fuzzy picker.
/// Here `root` is the top-level channel backing the user-facing workspace.
/// Includes a "＋ Create new channel…" entry at the top; selecting it prompts
/// for a name, creates the channel via the daemon, and returns the new id.
/// `agent_slug` is used as the default agent spec when creating.
async fn pick_channel(root: &str, agent_slug: &str) -> Result<String> {
    let v = super::super::daemon_call_async("channel_list", serde_json::json!({ "channel": root }))
        .await?;

    let rooms = v["rooms"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);

    // "＋ Create…" is always the first item so it's reachable by typing its name.
    const CREATE: &str = "＋  Create new channel…";
    let mut ids: Vec<Option<String>> = vec![None]; // None = create sentinel
    let mut labels: Vec<String> = vec![CREATE.to_string()];

    for r in rooms {
        let id = r["child_h"].as_str().unwrap_or("").to_string();
        let name = r["name"].as_str().unwrap_or("").to_string();
        let depth = r["depth"].as_u64().unwrap_or(0) as usize;
        let indent = "  ".repeat(depth);
        let label = if name.is_empty() {
            format!("{indent}{id}")
        } else {
            format!("{indent}{name}  ({})", &id[..id.len().min(12)])
        };
        labels.push(label);
        ids.push(Some(id));
    }

    let theme = dialoguer::theme::ColorfulTheme::default();
    let idx = dialoguer::FuzzySelect::with_theme(&theme)
        .with_prompt("Select channel")
        .items(&labels)
        .default(0)
        .interact()?;

    match &ids[idx] {
        Some(id) => Ok(id.clone()),
        None => create_channel_interactive(root, agent_slug, &theme).await,
    }
}

/// Prompt for a channel name, then create it via the daemon using the agent
/// being launched and the local backend pubkey. Returns the new channel id.
async fn create_channel_interactive(
    root: &str,
    agent_slug: &str,
    theme: &dialoguer::theme::ColorfulTheme,
) -> Result<String> {
    let name: String = dialoguer::Input::with_theme(theme)
        .with_prompt("Channel name")
        .interact_text()?;

    // Resolve the local backend config label from the daemon so the picker uses
    // the same backend identifier as `mosaico channel create --agent`.
    let backend_v = super::super::daemon_call_async("local_backend", serde_json::json!({})).await?;
    let backend_label = backend_v["backend_label"]
        .as_str()
        .context("local_backend did not return backend_label")?;

    let v = super::super::daemon_call_async(
        "channel_create",
        crate::cli::rpc_params(serde_json::json!({
            "parent": root,
            "name": &name,
            "about": &name,
            "agents": [{ "slug": agent_slug, "backend": backend_label }],
        })),
    )
    .await?;

    let child_h = v["child_h"]
        .as_str()
        .context("channel_create did not return child_h")?
        .to_string();
    eprintln!("created channel {child_h}");
    Ok(child_h)
}

#[cfg(test)]
mod tests {
    use super::channel_resolve_params;

    #[test]
    fn named_launch_channel_uses_channel_resolve_contract() {
        assert_eq!(
            channel_resolve_params("nmp", "forensic", "codex"),
            serde_json::json!({
                "channel": "nmp",
                "name": "forensic",
                "agent": "codex",
                "create_if_absent": true,
            })
        );
    }
}
