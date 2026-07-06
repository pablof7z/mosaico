use anyhow::{Context as _, Result};

// ── launch ───────────────────────────────────────────────────────────────────

/// Launch a fresh harness session and hand the current terminal to it.
///
/// Spawns an independent portable-pty supervisor, starts the selected harness
/// inside it, then attaches the current terminal to the new session.
pub(crate) async fn launch(
    agent: String,
    project: Option<String>,
    channel: Option<String>,
    command_name: Option<String>,
    override_command: Vec<String>,
    extra_args: Vec<String>,
) -> Result<()> {
    let project = match project {
        Some(p) => p,
        None => crate::project::resolve_or_bail(&std::env::current_dir().unwrap_or_default())?,
    };
    let base_command = if override_command.is_empty() {
        super::launch_command::resolve_launch_command(&agent, command_name.as_deref())?
    } else {
        override_command
    };
    // Show the interactive picker only when --channel "" is explicitly passed.
    // A bare `tenex-edge launch <agent>` with no --channel defaults to the
    // project root channel.
    let want_picker = matches!(channel, Some(ref s) if s.is_empty());
    let channel = if want_picker {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            anyhow::bail!(
                "channel selection needs a TTY to open the interactive picker; \
                 pass --channel <id> to scope into a specific channel non-interactively"
            );
        }
        Some(pick_channel(&project, &agent).await?)
    } else {
        channel
    };
    // Resolve a channel NAME (or a literal id) to its opaque `channel_h` BEFORE
    // spawning, so TENEX_EDGE_CHANNEL and provisioning both see ONE id (creating
    // it if absent). A picker selection is already an id and round-trips unchanged.
    // When no channel was specified, default to the project root channel.
    let channel = match channel {
        None => Some(project.clone()),
        Some(name) if !name.is_empty() => {
            let v = super::super::daemon_call_async(
                "channels_resolve",
                serde_json::json!({
                    "project": project.clone(),
                    "name": name,
                    "agent": agent.clone(),
                    "create_if_absent": true,
                }),
            )
            .await?;
            Some(
                v["channel_h"]
                    .as_str()
                    .context("channels_resolve did not return channel_h")?
                    .to_string(),
            )
        }
        other => other,
    };
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string());
    let mut command = base_command.clone();
    command.extend(extra_args);
    let cwd_path = cwd
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let meta = crate::pty::spawn_session(crate::pty::SpawnSessionArgs {
        id: None,
        agent: agent.clone(),
        project,
        cwd: cwd_path,
        channel,
        command,
    })?;
    eprintln!("[tenex-edge pty] session: {}", meta.id);
    eprintln!("[tenex-edge pty] detach: close this attach terminal");
    eprintln!(
        "[tenex-edge pty] reattach: tenex-edge pty attach {}",
        meta.id
    );
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        eprintln!("[tenex-edge pty] attach skipped: not running on a TTY");
        return Ok(());
    }
    crate::pty::attach(&meta.socket)
}

/// Fetch all rooms under `project` and present an interactive fuzzy picker.
/// Includes a "＋ Create new channel…" entry at the top; selecting it prompts
/// for a name, creates the channel via the daemon, and returns the new id.
/// `agent_slug` is used as the default agent spec when creating.
async fn pick_channel(project: &str, agent_slug: &str) -> Result<String> {
    let v =
        super::super::daemon_call_async("channels_list", serde_json::json!({ "project": project }))
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
        None => create_channel_interactive(project, agent_slug, &theme).await,
    }
}

/// Prompt for a channel name, then create it via the daemon using the agent
/// being launched and the local backend pubkey. Returns the new channel id.
async fn create_channel_interactive(
    project: &str,
    agent_slug: &str,
    theme: &dialoguer::theme::ColorfulTheme,
) -> Result<String> {
    let name: String = dialoguer::Input::with_theme(theme)
        .with_prompt("Channel name")
        .interact_text()?;

    // Resolve the local backend config label from the daemon so the picker uses
    // the same backend identifier as `tenex-edge channels create --agent`.
    let backend_v = super::super::daemon_call_async("local_backend", serde_json::json!({})).await?;
    let backend_label = backend_v["backend_label"]
        .as_str()
        .context("local_backend did not return backend_label")?;

    let v = super::super::daemon_call_async(
        "channels_create",
        crate::cli::rpc_params(serde_json::json!({
            "parent": project,
            "name": &name,
            "about": &name,
            "agents": [{ "slug": agent_slug, "backend": backend_label }],
        })),
    )
    .await?;

    let child_h = v["child_h"]
        .as_str()
        .context("channels_create did not return child_h")?
        .to_string();
    eprintln!("created channel {child_h}");
    Ok(child_h)
}
