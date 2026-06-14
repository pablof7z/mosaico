use super::*;

// ── tmux_run ──────────────────────────────────────────────────────────────────

/// Entry point for `tenex-edge tmux <action>`.
pub(super) async fn tmux_run(action: TmuxAction) -> Result<()> {
    match action {
        TmuxAction::Status => tmux_status().await,
        TmuxAction::Send { session } => tmux_send(session).await,
        TmuxAction::Spawn { agent, project } => tmux_spawn(agent, project).await,
        TmuxAction::Attach { session } => tmux_attach(session).await,
    }
}

// ── status ────────────────────────────────────────────────────────────────────

async fn tmux_status() -> Result<()> {
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

async fn tmux_send(session: String) -> Result<()> {
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

async fn tmux_spawn(agent: String, project: Option<String>) -> Result<()> {
    let project = project
        .unwrap_or_else(|| crate::project::resolve(&std::env::current_dir().unwrap_or_default()));
    let v = crate::daemon::blocking::call(
        "tmux_spawn",
        serde_json::json!({ "agent": agent, "project": project }),
    )
    .context("tmux_spawn RPC")?;

    let pane_id = v["pane_id"].as_str().unwrap_or("?");
    println!("Spawned pane {pane_id} for agent {agent} in project {project}.");
    Ok(())
}

// ── attach ────────────────────────────────────────────────────────────────────

async fn tmux_attach(session: String) -> Result<()> {
    attach_session(&session)
}

// ── shared attach logic ───────────────────────────────────────────────────────

fn attach_session(session_id: &str) -> Result<()> {
    let v =
        crate::daemon::blocking::call("tmux_attach", serde_json::json!({ "session": session_id }))
            .context("tmux_attach RPC")?;

    let pane_id = match v["pane_id"].as_str() {
        Some(p) => p.to_string(),
        None => {
            let err = v["error"].as_str().unwrap_or("unknown error");
            eprintln!("Cannot attach: {err}");
            return Ok(());
        }
    };

    attach_pane(&pane_id)
}

// ── TUI ───────────────────────────────────────────────────────────────────────

struct TuiTerminal;

impl TuiTerminal {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(Self)
    }
}

impl Drop for TuiTerminal {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
    }
}

struct LiveRow {
    slug: String,
    host: String,
    session_id: String,    // full raw id for RPC calls
    session_short: String, // short display code (6 chars)
    status: String,
    attachable: bool, // has a live tmux endpoint
}

struct SpawnRow {
    slug: String,
    host: String,
    command: String,
}

struct TuiData {
    live: Vec<LiveRow>,
    spawnable: Vec<SpawnRow>,
}

fn fetch_tui_data() -> Result<TuiData> {
    let v = crate::daemon::blocking::call(
        "who",
        serde_json::json!({
            "project": null,
            "all": false,
            "all_projects": true,
            "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
        }),
    )?;

    let live = v["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter(|r| !r["remote"].as_bool().unwrap_or(false))
        .map(|r| {
            let raw_id = r["session_id"].as_str().unwrap_or("").to_string();
            let session_short = SessionId::from(raw_id.as_str()).to_string();
            LiveRow {
                slug: r["slug"].as_str().unwrap_or("").to_string(),
                host: r["host"].as_str().unwrap_or("").to_string(),
                session_id: raw_id,
                session_short,
                status: r["status"].as_str().unwrap_or("").to_string(),
                attachable: r["attachable"].as_bool().unwrap_or(false),
            }
        })
        .collect();

    let spawnable = v["spawnable"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|r| SpawnRow {
            slug: r["slug"].as_str().unwrap_or("").to_string(),
            host: r["host"].as_str().unwrap_or("").to_string(),
            command: r["command"].as_str().unwrap_or("").to_string(),
        })
        .collect();

    Ok(TuiData { live, spawnable })
}

enum TuiExit {
    Quit,
    Attach(String),     // full session_id
    AttachPane(String), // direct pane_id (used after spawn)
}

fn draw_tui(data: &TuiData, selected: usize, status: &str) -> Result<()> {
    use owo_colors::OwoColorize as _;

    let mut out = String::new();
    let _ = writeln!(out, "{}", "tenex-edge tmux".bold());
    let _ = writeln!(out, "{}", "─".repeat(60).dimmed());
    let _ = writeln!(out);

    let _ = writeln!(out, "  {}", "Live sessions".bold());
    if data.live.is_empty() {
        let _ = writeln!(out, "    {}", "(none)".dimmed());
    } else {
        for (i, row) in data.live.iter().enumerate() {
            let cursor = if i == selected { "►" } else { " " };
            let label = format!("{}@{}", row.slug, row.host);
            let session_tag = format!("[session {}]", row.session_short);
            let status_str = if row.status.trim().is_empty() {
                "idle".to_string()
            } else {
                row.status.trim().to_string()
            };
            if !row.attachable {
                // Not running in tmux — show dimmed with [no tmux] marker.
                let _ = writeln!(
                    out,
                    "  {} {}  {}  {} {}",
                    cursor,
                    label.dimmed(),
                    session_tag.dimmed(),
                    status_str.dimmed(),
                    "[no tmux]".dimmed(),
                );
            } else if i == selected {
                let _ = writeln!(
                    out,
                    "  {} {}  {}  {}",
                    cursor,
                    label.cyan().bold(),
                    session_tag.yellow(),
                    status_str,
                );
            } else {
                let _ = writeln!(
                    out,
                    "  {} {}  {}  {}",
                    cursor,
                    label.cyan(),
                    session_tag.yellow(),
                    status_str.dimmed(),
                );
            }
        }
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "  {}", "Spawnable (no session)".bold());
    if data.spawnable.is_empty() {
        let _ = writeln!(out, "    {}", "(none)".dimmed());
    } else {
        for (i, row) in data.spawnable.iter().enumerate() {
            let abs_idx = data.live.len() + i;
            let cursor = if abs_idx == selected { "►" } else { " " };
            let label = format!("{}@{}", row.slug, row.host);
            let tag = format!("[spawnable via {}]", row.command);
            if abs_idx == selected {
                let _ = writeln!(out, "  {} {}  {}", cursor, label.bold(), tag.dimmed(),);
            } else {
                let _ = writeln!(out, "  {} {}  {}", cursor, label.dimmed(), tag.dimmed(),);
            }
        }
    }

    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "  {}",
        "[↑↓/jk] move   [a/↵] attach   [n] spawn   [q] quit".dimmed()
    );
    if !status.is_empty() {
        let _ = writeln!(out, "  {status}");
    }

    let mut stdout = io::stdout();
    execute!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;
    for line in out.lines() {
        write!(stdout, "{line}\r\n")?;
    }
    stdout.flush()?;
    Ok(())
}

/// Interactive TUI for `tenex-edge tmux` (bare, no subcommand).
/// Shows live sessions and spawnable agents; lets the user attach or spawn.
pub(super) fn tmux_tui() -> Result<()> {
    let refresh = Duration::from_secs(2);
    let mut selected: usize = 0;
    let mut status_msg = String::new();

    // Initial fetch before entering raw mode: fail fast if daemon is down.
    let mut data = fetch_tui_data()?;

    let exit_action = {
        let _terminal = TuiTerminal::enter()?;
        let mut next_refresh = Instant::now() + refresh;
        let mut result = TuiExit::Quit;

        loop {
            let total = data.live.len() + data.spawnable.len();
            if total > 0 && selected >= total {
                selected = total - 1;
            }

            draw_tui(&data, selected, &status_msg)?;

            let wait = next_refresh
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(100));

            if event::poll(wait)? {
                if let TermEvent::Key(key) = event::read()? {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            break
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            selected = selected.saturating_sub(1);
                            status_msg.clear();
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if total > 0 && selected + 1 < total {
                                selected += 1;
                            }
                            status_msg.clear();
                        }
                        KeyCode::Enter | KeyCode::Char('a') => {
                            if selected < data.live.len() {
                                let row = &data.live[selected];
                                if row.attachable {
                                    result = TuiExit::Attach(row.session_id.clone());
                                    break;
                                } else {
                                    status_msg =
                                        "Session not running in tmux — cannot attach.".to_string();
                                }
                            } else {
                                // Selected item is a spawnable — hint to use 'n'.
                                status_msg = "Press [n] to spawn this agent.".to_string();
                            }
                        }
                        KeyCode::Char('n') if selected >= data.live.len() => {
                            let spawnable_idx = selected - data.live.len();
                            if spawnable_idx < data.spawnable.len() {
                                let slug = data.spawnable[spawnable_idx].slug.clone();
                                let project = crate::project::resolve(
                                    &std::env::current_dir().unwrap_or_default(),
                                );
                                status_msg = format!("Spawning {slug}...");
                                draw_tui(&data, selected, &status_msg)?;
                                match crate::daemon::blocking::call(
                                    "tmux_spawn",
                                    serde_json::json!({
                                        "agent": slug,
                                        "project": project,
                                    }),
                                ) {
                                    Ok(v) => {
                                        let pane = v["pane_id"].as_str().unwrap_or("?").to_string();
                                        // Switch directly to the new pane.
                                        result = TuiExit::AttachPane(pane);
                                        break;
                                    }
                                    Err(e) => {
                                        status_msg = format!("Spawn failed: {e}");
                                        // Refresh immediately after failed spawn attempt.
                                        if let Ok(fresh) = fetch_tui_data() {
                                            data = fresh;
                                        }
                                        next_refresh = Instant::now() + refresh;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            // Periodic refresh.
            if Instant::now() >= next_refresh {
                if let Ok(fresh) = fetch_tui_data() {
                    data = fresh;
                }
                next_refresh = Instant::now() + refresh;
            }
        }

        result
    }; // _terminal dropped here — raw mode disabled, alternate screen exited

    match exit_action {
        TuiExit::Attach(session_id) => attach_session(&session_id),
        TuiExit::AttachPane(pane_id) => attach_pane(&pane_id),
        TuiExit::Quit => Ok(()),
    }
}

fn attach_pane(pane_id: &str) -> Result<()> {
    let in_tmux = std::env::var("TMUX").is_ok();
    if in_tmux {
        // switch-client can jump to any pane in any window/session, unlike
        // select-pane which only works within the currently focused window.
        let status = std::process::Command::new("tmux")
            .args(["switch-client", "-t", pane_id])
            .status()
            .context("tmux switch-client")?;
        if status.success() {
            return Ok(());
        }
        eprintln!("tmux switch-client failed for pane {pane_id}");
        return Ok(());
    }

    // Not inside tmux: find which session:window owns this pane, then exec
    // attach-session so this process is replaced by the tmux client.
    let out = std::process::Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{pane_id} #{session_name}:#{window_index}",
        ])
        .output()
        .context("tmux list-panes")?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let target = stdout.lines().find_map(|line| {
        let (pid, rest) = line.split_once(' ')?;
        if pid == pane_id {
            Some(rest.to_string())
        } else {
            None
        }
    });

    match target {
        Some(t) => {
            use std::os::unix::process::CommandExt;
            let err = std::process::Command::new("tmux")
                .args(["attach-session", "-t", &t])
                .exec(); // replaces this process; only returns on error
            anyhow::bail!("exec tmux attach-session: {err}");
        }
        None => {
            eprintln!(
                "Pane {pane_id} not found in any tmux session. Run: tmux attach-session -t tenex"
            );
            Ok(())
        }
    }
}
