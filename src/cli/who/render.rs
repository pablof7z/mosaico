use super::*;

pub(super) fn render_who_once(snapshot: &WhoSnapshot) -> String {
    let mut out = String::new();

    let scope = if snapshot.project == "*" {
        "all projects".to_string()
    } else {
        snapshot.project.clone()
    };
    let _ = writeln!(out, "{}", scope.bold());
    let _ = writeln!(out);

    if snapshot.rows.is_empty() {
        let _ = writeln!(
            out,
            "(no live agents{})",
            if snapshot.all {
                ""
            } else {
                " — start a session, or run with --all to include stale"
            }
        );
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
            let label = format!("{}@{}", row.slug, row.host);
            let tag = format!("[spawnable via {}]", row.command);
            let _ = writeln!(out, "{}  {}", label.dimmed(), tag.dimmed());
        }
    }

    out
}

pub(super) fn render_who_plain(snapshot: &WhoSnapshot) -> String {
    strip_ansi(&render_who_once(snapshot))
}

fn render_who_row(out: &mut String, row: &WhoRow, include_project: bool) {
    let stale = if row.fresh {
        String::new()
    } else {
        format!(" {}", "(stale)".dimmed())
    };
    // Always show which host the agent runs on. Same-machine agents get a plain
    // `(hostname)`; a true remote (peer on a different host than the daemon) is
    // flagged `(hostname, remote)` so cross-machine sessions stay distinguishable.
    let host = if row.remote {
        format!(" {}", format!("({}, remote)", row.host).dimmed())
    } else {
        format!(" {}", format!("({})", row.host).dimmed())
    };
    let dir = rel_cwd_bracket(&row.rel_cwd)
        .map(|d| format!(" {}", format!("[{d}]").dimmed()))
        .unwrap_or_default();
    let unread = if row.unread > 0 {
        format!(" {}", format!("◉{}", row.unread).yellow())
    } else {
        String::new()
    };
    let name = if include_project {
        format!("{}@{}", row.slug, row.project).cyan().to_string()
    } else {
        row.slug.cyan().to_string()
    };
    let _ = writeln!(
        out,
        "{} [session {}]{}{}{}{} - {}",
        name,
        session_short_code(&row.session_id).yellow(),
        dir,
        host,
        stale,
        unread,
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

fn strip_ansi(input: &str) -> String {
    let mut out = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}
