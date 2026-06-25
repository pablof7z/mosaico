/// CLI verb handlers for tmux commands
use anyhow::{Context as _, Result};

// ── status ────────────────────────────────────────────────────────────────────

pub(super) async fn tmux_status() -> Result<()> {
    use owo_colors::OwoColorize as _;

    let v = crate::daemon::blocking::call("tmux_status", serde_json::json!({}))
        .context("tmux_status RPC")?;

    let endpoints = v["endpoints"].as_array().cloned().unwrap_or_default();

    if endpoints.is_empty() {
        println!("No tmux endpoints registered.");
        return Ok(());
    }

    println!(
        "{:<22} {:<8} {:<12} {}",
        "session".bold(),
        "pane".bold(),
        "command".bold(),
        "alive".bold()
    );
    for ep in &endpoints {
        let sid = ep["session_id"].as_str().unwrap_or("");
        let pane = ep["pane_id"].as_str().unwrap_or("");
        let cmd = ep["pane_command"].as_str().unwrap_or("");
        let alive = ep["alive"].as_bool().unwrap_or(false);
        let alive_str = if alive {
            "yes".green().to_string()
        } else {
            "DEAD".red().to_string()
        };
        println!("{sid:<22} {pane:<8} {cmd:<12} {alive_str}");
    }
    Ok(())
}

// ── send (manual doorbell) ────────────────────────────────────────────────────

pub(super) async fn tmux_send(session: String) -> Result<()> {
    let v = crate::daemon::blocking::call("tmux_send", serde_json::json!({ "session": session }))
        .context("tmux_send RPC")?;

    let injected = v["injected"].as_bool().unwrap_or(false);
    if injected {
        println!("Doorbell injected.");
    } else {
        let reason = v["reason"].as_str().unwrap_or("unknown");
        println!("Doorbell not sent: {reason}");
    }
    Ok(())
}

// ── spawn ─────────────────────────────────────────────────────────────────────

pub(super) async fn tmux_spawn(agent: String, project: Option<String>) -> Result<()> {
    let project = match project {
        Some(p) => p,
        None => crate::project::resolve_or_bail(&std::env::current_dir().unwrap_or_default())?,
    };
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string());
    let v = crate::daemon::blocking::call(
        "tmux_spawn",
        serde_json::json!({ "agent": agent, "project": project, "command": [], "cwd": cwd }),
    )
    .context("tmux_spawn RPC")?;

    let pane_id = v["pane_id"].as_str().unwrap_or("?");
    println!("Spawned pane {pane_id} for agent {agent} in project {project}.");
    Ok(())
}

// ── launch ───────────────────────────────────────────────────────────────────

/// Launch a fresh harness session and hand the current terminal to it.
///
/// Identical to `tmux_spawn` (same `tmux_spawn` RPC, same transparent-session
/// options applied inside `open_agent_session`) — the only difference is that
/// `launch` then attaches the current terminal to the new session, while
/// `tmux_spawn` just prints the pane id. Both paths produce a session with the
/// tmux chrome already hidden and the prefix key unbound, so no per-verb
/// `hide_session_chrome` step is needed here.
pub(crate) async fn launch(
    agent: String,
    project: Option<String>,
    channel: Option<String>,
    override_command: Vec<String>,
    extra_args: Vec<String>,
) -> Result<()> {
    let project = match project {
        Some(p) => p,
        None => crate::project::resolve_or_bail(&std::env::current_dir().unwrap_or_default())?,
    };
    // `--channel` with no value → open the interactive picker.
    let channel = match channel {
        Some(ref s) if s.is_empty() => {
            use std::io::IsTerminal;
            if !std::io::stdin().is_terminal() {
                anyhow::bail!(
                    "--channel with no value opens an interactive picker that needs a TTY; \
                     pass --channel <id> to scope into a specific channel non-interactively"
                );
            }
            Some(pick_channel(&project, &agent).await?)
        }
        other => other,
    };
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string());
    let v = crate::daemon::blocking::call(
        "tmux_spawn",
        serde_json::json!({
            "agent": agent,
            "project": project,
            "channel": channel,
            "command": extra_args,
            "base_command": override_command,
            "cwd": cwd,
        }),
    )
    .context("tmux_spawn RPC")?;

    let pane_id = v["pane_id"]
        .as_str()
        .context("tmux_spawn response did not include pane_id")?;
    crate::cli::tmux_cli::attach::attach_pane(pane_id)
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

    // Resolve the local backend pubkey from the daemon so we don't have to
    // guess the hostname format the daemon uses internally.
    let backend_v = super::super::daemon_call_async("local_backend", serde_json::json!({})).await?;
    let backend_pubkey = backend_v["pubkey"]
        .as_str()
        .context("local_backend did not return pubkey")?;

    let v = super::super::daemon_call_async(
        "channels_create",
        serde_json::json!({
            "parent": project,
            "name": name,
            "agents": [{ "slug": agent_slug, "backend": backend_pubkey }],
            "brief": "",
            "agent": crate::cli::agent_env_slug(),
            "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
            "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
        }),
    )
    .await?;

    let child_h = v["child_h"]
        .as_str()
        .context("channels_create did not return child_h")?
        .to_string();
    eprintln!("created channel {child_h}");
    Ok(child_h)
}

// ── attach ────────────────────────────────────────────────────────────────────

pub(super) async fn tmux_attach(session: String) -> Result<()> {
    super::attach::attach_session(&session)
}

// ── resume ────────────────────────────────────────────────────────────────────

pub(super) async fn tmux_resume(session: String) -> Result<()> {
    let pane = super::attach::resume_to_pane(&session)?;
    match pane {
        Some(pane_id) => super::attach::attach_pane(&pane_id),
        None => Ok(()),
    }
}

/// Session id of the currently-selected row IF it is resumable — any local Live
/// row (attachable or not: an in-tmux session can still be replayed) or any
/// Resumable row. `None` for Spawnable rows. The daemon makes the final call on
/// whether a token exists; this just maps cursor → session id.
pub(super) fn selected_resume_sid(
    live: &[&super::tui_model::LiveRow],
    spawnable_count: usize,
    resumable: &[&super::tui_model::ResumeRow],
    selected: usize,
) -> Option<String> {
    if selected < live.len() {
        return Some(live[selected].session_id.clone());
    }
    let resume_base = live.len() + spawnable_count;
    if selected >= resume_base {
        return resumable
            .get(selected - resume_base)
            .map(|r| r.session_id.clone());
    }
    None
}
