use super::*;

pub(super) fn render_who_once(snapshot: &WhoSnapshot) -> String {
    let mut out = String::new();
    let name_counts = agent_name_counts(&snapshot.rows);

    let scope = if snapshot.project == "*" {
        "all projects".to_string()
    } else {
        snapshot.project.clone()
    };
    let _ = writeln!(out, "{}", scope.bold());
    let _ = writeln!(out);

    if snapshot.rows.is_empty() {
        let _ = writeln!(out, "(no live agents — start a session)");
    } else if snapshot.project == "*" {
        for row in &snapshot.rows {
            render_who_row(&mut out, row, true, &name_counts);
        }
    } else {
        for row in &snapshot.rows {
            render_who_row(&mut out, row, false, &name_counts);
        }
    }

    if snapshot.project != "*" && !snapshot.other_projects.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "{} other agent(s) in other projects:",
            snapshot.other_projects.len()
        );
        for op in &snapshot.other_projects {
            match &op.about {
                Some(about) if !about.is_empty() => {
                    let _ = writeln!(out, "  * {} - {}", op.project, about);
                }
                _ => {
                    let _ = writeln!(out, "  * {}", op.project);
                }
            }
        }
    }

    if !snapshot.spawnable.is_empty() {
        let _ = writeln!(out);
        for row in &snapshot.spawnable {
            let label = crate::idref::agent_label(&row.slug, &row.host);
            let byline = match row.byline.as_deref().map(str::trim) {
                Some(b) if !b.is_empty() => format!(" — {b}"),
                _ => String::new(),
            };
            let tag = format!("[spawnable via {}]", row.command);
            let _ = writeln!(out, "{}{}  {}", label.dimmed(), byline, tag.dimmed());
        }
    }

    out
}

/// Render the `whoami` identity card from the daemon's JSON. Auto-detects a TTY:
/// terminal → a compact colorized block; piped/captured (the agent-facing case)
/// → a plain markdown table so it parses cleanly when injected into context.
pub(super) fn render_whoami(v: &serde_json::Value) -> String {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let agent = s("agent");
    let project = s("project");
    let host = s("host");
    let rel_cwd = s("rel_cwd");
    let pubkey = s("pubkey");
    let working = v.get("working").and_then(|x| x.as_bool()).unwrap_or(false);
    let title = s("status");
    let is_member = v.get("is_member").and_then(|x| x.as_bool()).unwrap_or(true);
    let pending = v.get("pending").and_then(|x| x.as_u64()).unwrap_or(0);

    let status = status_plain(&title, "", working);
    // The durable agent pubkey is the wire address others route to. Never npub.
    let key = pubkey;
    let dir = if rel_cwd.trim().is_empty() || rel_cwd == "." {
        host.clone()
    } else {
        format!("{host} [{rel_cwd}]")
    };

    if io::stdout().is_terminal() {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "You are {} on {}.",
            agent.cyan().bold(),
            project
        );
        let _ = writeln!(out);
        let row = |k: &str, val: &str| format!("  {:<10} {}\n", format!("{k}:").dimmed(), val);
        let _ = write!(out, "{}", row("agent", &agent));
        let _ = write!(out, "{}", row("project", &project));
        let _ = write!(out, "{}", row("host", &dir));
        let _ = write!(out, "{}", row("pubkey", &key));
        let _ = write!(out, "{}", row("status", &status));
        let _ = write!(
            out,
            "{}",
            row("member", if is_member { "yes" } else { "no" })
        );
        if pending > 0 {
            let _ = write!(out, "{}", row("chat", &format!("{pending} pending")));
        }
        out
    } else {
        let mut out = String::new();
        let _ = writeln!(out, "You are **{agent}** on **{project}**.");
        let _ = writeln!(out);
        let _ = writeln!(out, "| Field | Value |");
        let _ = writeln!(out, "|---|---|");
        let _ = writeln!(out, "| Agent | {} |", md_cell(&agent));
        let _ = writeln!(out, "| Project | {} |", md_cell(&project));
        let _ = writeln!(out, "| Host | {} |", md_cell(&dir));
        let _ = writeln!(out, "| Pubkey | {} |", md_cell(&key));
        let _ = writeln!(out, "| Status | {} |", md_cell(&status));
        let _ = writeln!(
            out,
            "| Project member | {} |",
            if is_member { "yes" } else { "no" }
        );
        if pending > 0 {
            let _ = writeln!(out, "| Chat | {} pending |", pending);
        }
        out
    }
}

pub(super) fn render_who_for_stdout(snapshot: &WhoSnapshot) -> String {
    if io::stdout().is_terminal() {
        render_who_once(snapshot)
    } else {
        render_who_plain(snapshot)
    }
}

pub(super) fn render_who_plain(snapshot: &WhoSnapshot) -> String {
    let mut out = String::new();
    let name_counts = agent_name_counts(&snapshot.rows);

    let _ = writeln!(out, "# tenex-edge who");
    let _ = writeln!(out);
    if snapshot.project == "*" {
        let _ = writeln!(out, "Project: all projects");
    } else if let Some(parent) = &snapshot.channel_parent {
        // The current scope is this session's own room — show it as the channel,
        // with the work-root project it's nested under.
        let _ = writeln!(out, "Channel: {} (your session room)", snapshot.project);
        let _ = writeln!(out, "Project: {parent}");
    } else {
        let _ = writeln!(out, "Project: {}", snapshot.project);
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## Agents in this channel");
    let _ = writeln!(
        out,
        "Use `tenex-edge chat write --message \"...\"` to write to this channel."
    );
    let _ = writeln!(out);
    if snapshot.rows.is_empty() {
        let _ = writeln!(out, "_No live agents visible._");
    } else {
        for line in AGENT_TABLE_HEADER {
            let _ = writeln!(out, "{line}");
        }
        for row in &snapshot.rows {
            render_who_markdown_row(&mut out, row, snapshot.project == "*", &name_counts);
        }
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "## Agents (for new sessions)");
    let _ = writeln!(
        out,
        "Start a new session with `tenex-edge chat write --message \"...\"`."
    );
    let _ = writeln!(out);
    if snapshot.spawnable.is_empty() {
        let _ = writeln!(out, "_No local spawnable agents configured._");
    } else {
        let _ = writeln!(out, "| Agent | Host | When to use |");
        let _ = writeln!(out, "|---|---|---|");
        for row in &snapshot.spawnable {
            let byline = row.byline.as_deref().map(md_cell).unwrap_or_default();
            let _ = writeln!(
                out,
                "| {} | {} | {} |",
                md_cell(&row.slug),
                md_cell(&row.host),
                byline
            );
        }
    }

    if snapshot.project != "*" {
        let _ = writeln!(out);
        let _ = writeln!(out, "## Other projects");
        if snapshot.other_projects.is_empty() {
            let _ = writeln!(out);
            let _ = writeln!(out, "_No other projects visible._");
        } else {
            let _ = writeln!(out);
            for op in &snapshot.other_projects {
                let _ = writeln!(out, "- {}", md_text(&op.project));
            }
        }
    }

    out
}

fn render_who_markdown_row(
    out: &mut String,
    row: &WhoRow,
    _include_project: bool,
    name_counts: &std::collections::BTreeMap<String, usize>,
) {
    let agent = display_agent_name(&row.slug, &row.session_id, name_counts);
    let host = row_host_label(row);
    let title = row_title_label(row);
    let status = row_state_label(row);
    let _ = writeln!(
        out,
        "| {} | {} | {} | {} |",
        md_cell(&agent),
        md_cell(&host),
        md_cell(&title),
        md_cell(&status)
    );
}

pub(super) fn agent_name_counts(rows: &[WhoRow]) -> std::collections::BTreeMap<String, usize> {
    let mut counts = std::collections::BTreeMap::new();
    for row in rows {
        *counts.entry(row.slug.clone()).or_insert(0) += 1;
    }
    counts
}

pub(super) fn display_agent_name(
    slug: &str,
    session_id: &str,
    name_counts: &std::collections::BTreeMap<String, usize>,
) -> String {
    if name_counts.get(slug).copied().unwrap_or(0) > 1 {
        format!("{}-{}", slug, session_codename(session_id))
    } else {
        slug.to_string()
    }
}

fn row_host_label(row: &WhoRow) -> String {
    let host = if row.remote {
        format!("{}, remote", crate::util::slugify_host(&row.host))
    } else {
        crate::util::slugify_host(&row.host)
    };
    rel_cwd_bracket(&row.rel_cwd)
        .map(|dir| format!("{host} [{dir}]"))
        .unwrap_or(host)
}

fn row_title_label(row: &WhoRow) -> String {
    if row.status.trim().is_empty() {
        "—".to_string()
    } else {
        row.status.trim().to_string()
    }
}

fn row_state_label(row: &WhoRow) -> String {
    let mut status = if row.active {
        let activity = row.activity.trim();
        if activity.is_empty() {
            "working".to_string()
        } else {
            activity.to_string()
        }
    } else {
        "idle".to_string()
    };
    if !row.fresh {
        status.push_str(" (stale)");
    }
    status
}

const AGENT_TABLE_HEADER: [&str; 2] = ["| Agent | Host | Title | Status |", "|---|---|---|---|"];

fn md_cell(input: &str) -> String {
    md_text(input).replace('|', r"\|")
}

fn md_text(input: &str) -> String {
    input
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn render_who_row(
    out: &mut String,
    row: &WhoRow,
    include_project: bool,
    name_counts: &std::collections::BTreeMap<String, usize>,
) {
    let stale = if row.fresh {
        String::new()
    } else {
        format!(" {}", "(stale)".dimmed())
    };
    // Always show which host the agent runs on. A true remote peer is flagged so
    // cross-machine sessions stay distinguishable without exposing a session id.
    let host = if row.remote {
        format!("{}, remote", crate::util::slugify_host(&row.host))
    } else {
        crate::util::slugify_host(&row.host)
    };
    let dir = rel_cwd_bracket(&row.rel_cwd)
        .map(|d| format!(" {}", format!("[{d}]").dimmed()))
        .unwrap_or_default();
    let _ = include_project;
    let name = display_agent_name(&row.slug, &row.session_id, name_counts)
        .cyan()
        .to_string();
    let _ = writeln!(
        out,
        "{} ({}){}{} - {}",
        name,
        host,
        dir,
        stale,
        status_colored(&row.status, &row.activity, row.active),
    );
}

/// The `[dir]` to show for a row's `rel_cwd`: `None` when empty or the project
/// root (`.`), so the project root renders without a bracket (§8e).
fn rel_cwd_bracket(rel_cwd: &str) -> Option<&str> {
    let r = rel_cwd.trim();
    if r.is_empty() || r == "." {
        None
    } else {
        Some(r)
    }
}

pub(super) fn draw_who_live(snapshot: &WhoSnapshot, refresh: Duration) -> Result<()> {
    let refresh_ms = refresh.as_millis();
    let mut screen = render_who_once(snapshot);
    let _ = writeln!(
        screen,
        "{}",
        format!("  --live  refresh {refresh_ms}ms  q/esc/ctrl-c to quit").dimmed()
    );
    let mut stdout = io::stdout();
    execute!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;
    for line in screen.lines() {
        write!(stdout, "{line}\r\n")?;
    }
    stdout.flush()?;
    Ok(())
}

/// Plain (no-ANSI) status label: the persistent title, the live activity while
/// mid-turn (`title — activity`), and an idle marker when not mid-turn. Used for
/// injected context blocks where ANSI must not leak. Empty title falls back to
/// the activity, then to a bare "working"/"idle" word.
pub(super) fn status_plain(title: &str, activity: &str, active: bool) -> String {
    let t = title.trim();
    let a = activity.trim();
    match (t.is_empty(), active) {
        (true, true) if !a.is_empty() => a.to_string(),
        (true, true) => "working".to_string(),
        (true, false) => "idle".to_string(),
        (false, true) if !a.is_empty() => format!("{t} — {a}"),
        (false, true) => t.to_string(),
        (false, false) => format!("{t} · idle"),
    }
}

/// Terminal status label: like [`status_plain`] but dims the live activity and
/// the idle marker so the persistent title stays prominent.
fn status_colored(title: &str, activity: &str, active: bool) -> String {
    let t = title.trim();
    let a = activity.trim();
    match (t.is_empty(), active) {
        (true, true) if !a.is_empty() => a.dimmed().to_string(),
        (true, true) => "working".dimmed().to_string(),
        (true, false) => "idle".dimmed().to_string(),
        (false, true) if !a.is_empty() => format!("{} {}", t, format!("— {a}").dimmed()),
        (false, true) => t.to_string(),
        (false, false) => format!("{} {}", t, "· idle".dimmed()),
    }
}

pub(super) fn should_quit_live(event: TermEvent) -> bool {
    let TermEvent::Key(key) = event else {
        return false;
    };
    matches!(key.code, KeyCode::Esc | KeyCode::Char('q'))
        || (matches!(key.code, KeyCode::Char('c')) && key.modifiers.contains(KeyModifiers::CONTROL))
}

pub(super) struct LiveTerminal;

impl LiveTerminal {
    pub(super) fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, Hide)?;
        Ok(Self)
    }
}

impl Drop for LiveTerminal {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
    }
}
