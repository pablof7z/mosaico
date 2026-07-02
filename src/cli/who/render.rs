use super::snapshot::WhoRow;
use super::*;

pub(super) fn render_who_once(snapshot: &WhoSnapshot) -> String {
    let mut out = String::new();

    let scope = if snapshot.project == "*" {
        "all projects".to_string()
    } else {
        snapshot.project_display.clone()
    };
    let _ = writeln!(out, "{}", scope.bold());
    let _ = writeln!(out);

    if snapshot.rows.is_empty() {
        let _ = writeln!(out, "(no live agents — start a session)");
    } else if snapshot.project == "*" {
        for row in &snapshot.rows {
            render_who_row(&mut out, row, true);
        }
    } else {
        for row in &snapshot.rows {
            render_who_row(&mut out, row, false);
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

/// A concise self-identity header folded into `who` (issue #99): who you are on
/// the fabric. Driven by `out["self"]`; `None` when `who` is not run inside an
/// agent. The agent label is the user-facing identity.
pub(super) fn render_self_header(v: &serde_json::Value) -> Option<String> {
    let me = v.get("self")?;
    let s = |k: &str| me.get(k).and_then(|x| x.as_str()).unwrap_or("");
    let label = s("label");
    if label.is_empty() {
        return None;
    }
    let host = s("host");
    let pubkey = s("pubkey");
    let working = me.get("working").and_then(|x| x.as_bool()).unwrap_or(false);
    let status = status_plain(s("status"), "", working);
    let is_member = me
        .get("is_member")
        .and_then(|x| x.as_bool())
        .unwrap_or(true);
    let pending = me.get("pending").and_then(|x| x.as_u64()).unwrap_or(0);

    let _ = pubkey; // not shown to the agent
    let mut out = if host.is_empty() {
        format!("You are **{label}**.")
    } else {
        format!("You are **{label}** on **{host}**.")
    };
    let _ = write!(out, "\nstatus {status}");
    if !is_member {
        let _ = write!(out, " · not a member of this channel");
    }
    if pending > 0 {
        let _ = write!(out, " · {pending} pending");
    }
    Some(out)
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

    let _ = writeln!(out, "# tenex-edge who");
    let _ = writeln!(out);
    if snapshot.project == "*" {
        let _ = writeln!(out, "Project: all projects");
    } else if let Some(parent) = &snapshot.channel_parent {
        // The current scope is this session's own room — show it as the channel,
        // with the work-root project it's nested under.
        let _ = writeln!(
            out,
            "Channel: {} (your session room)",
            snapshot.project_display
        );
        let _ = writeln!(out, "Project: {parent}");
    } else {
        let _ = writeln!(out, "Project: {}", snapshot.project_display);
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
            render_who_markdown_row(&mut out, row, snapshot.project == "*");
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

fn render_who_markdown_row(out: &mut String, row: &WhoRow, _include_project: bool) {
    // Concurrent same-agent instances now carry DISTINCT ordinal slugs
    // ("haiku"/"haiku1"), so the slug alone disambiguates.
    let agent = row.slug.clone();
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

fn row_host_label(row: &WhoRow) -> String {
    let host = row.host.trim();
    let host = if row.remote {
        format!("{host}, remote")
    } else {
        host.to_string()
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

fn render_who_row(out: &mut String, row: &WhoRow, include_project: bool) {
    let stale = if row.fresh {
        String::new()
    } else {
        format!(" {}", "(stale)".dimmed())
    };
    // Always show which host the agent runs on. A true remote peer is flagged so
    // cross-machine sessions stay distinguishable without exposing a session id.
    let host = row.host.trim();
    let host = if row.remote {
        format!("{host}, remote")
    } else {
        host.to_string()
    };
    let dir = rel_cwd_bracket(&row.rel_cwd)
        .map(|d| format!(" {}", format!("[{d}]").dimmed()))
        .unwrap_or_default();
    let _ = include_project;
    // Distinct ordinal slugs already disambiguate concurrent same-agent rows.
    let name = row.slug.clone().cyan().to_string();
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

/// Live-redraw a pre-rendered fabric string (the unified `who` view) in the
/// alternate screen, same chrome as [`draw_who_live`].
pub(super) fn draw_fabric_live(text: &str, refresh: Duration) -> Result<()> {
    let refresh_ms = refresh.as_millis();
    let mut screen = text.to_string();
    let _ = writeln!(
        screen,
        "\n{}",
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
