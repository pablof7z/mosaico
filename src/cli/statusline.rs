//! `tenex-edge statusline` — the fabric, one line at a time.
//!
//! Renders the awareness floor for a host status bar:
//!   claude@kubrick [601a36] ⬡4 ◉9 ✎ refactoring inbox ✉ codex@kubrick: review?
//!   └ identity ┘  └session┘ │   │  └ own session status┘ └ inbox envelope ┘
//!                  members ─┘   └─ live sessions (incl. idle)
//!
//! Reads the harness's statusline JSON payload on stdin (Claude Code sends
//! `session_id` + `workspace.current_dir`), asks the daemon for one pure-read
//! snapshot, prints one line. Harnesses re-run this constantly, so it must fail
//! open — daemon down → print nothing, exit 0, and NEVER spawn a daemon just to
//! draw a line.

use super::*;

/// Cap for the own-status segment.
const STATUS_MAX_CHARS: usize = 48;
/// Cap for the inbox message preview.
const MESSAGE_MAX_CHARS: usize = 60;

pub(super) fn statusline(
    session: Option<String>,
    agent_arg: Option<String>,
    cwd_arg: Option<String>,
    tmux_fmt: bool,
) -> Result<()> {
    // Harness payload on stdin (absent when invoked by hand from a terminal or
    // from the tmux status-format #(...) invocation).
    let raw: serde_json::Value = if io::stdin().is_terminal() {
        serde_json::Value::Null
    } else {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf).ok();
        serde_json::from_str(&buf).unwrap_or(serde_json::Value::Null)
    };
    let env_session = raw
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    // Explicit --cwd wins, then stdin payload, then process cwd.
    let cwd = cwd_arg
        .or_else(|| {
            raw.pointer("/workspace/current_dir")
                .or_else(|| raw.get("cwd"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        });
    // Explicit --agent wins over the env-var slug (the tmux status-format
    // invocation passes --agent because TENEX_EDGE_AGENT is a pane env var
    // not available in the tmux server's #(...) execution context).
    let agent = agent_arg.or_else(agent_env_slug);

    let params = serde_json::json!({
        "session": session,
        "env_session": env_session,
        "cwd": cwd,
        "agent": agent,
        "channel": crate::cli::channel_env(),
    });
    // Fail open on ANY failure (no daemon, no session yet, protocol skew): a
    // status bar with a missing segment beats a status bar with an error in it.
    let Ok(v) = crate::daemon::blocking::call_no_spawn("statusline", params) else {
        return Ok(());
    };
    let Ok(view) = serde_json::from_value::<StatuslineView>(v) else {
        return Ok(());
    };
    let line = if tmux_fmt {
        render_statusline_tmux(&view)
    } else {
        render_statusline(&view, true)
    };
    println!("{line}");
    Ok(())
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct StatuslineView {
    #[serde(default)]
    agent: String,
    #[serde(default)]
    host: String,
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    project: String,
    #[serde(default)]
    member_count: u64,
    #[serde(default)]
    session_count: u64,
    #[serde(default = "default_true")]
    is_member: bool,
    #[serde(default)]
    working: bool,
    #[serde(default)]
    status: String,
    #[serde(default)]
    pending: Vec<MentionView>,
    #[serde(default)]
    recent: Vec<MentionView>,
    #[serde(default)]
    distill_error: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct MentionView {
    #[serde(default)]
    from_slug: String,
    #[serde(default)]
    host: String,
    #[serde(default)]
    subject: String,
    #[serde(default)]
    body: String,
}

impl MentionView {
    /// `codex@kubrick: Please review my work` — subject wins over body.
    fn preview(&self) -> String {
        let text = if self.subject.is_empty() {
            self.body.lines().next().unwrap_or_default()
        } else {
            self.subject.as_str()
        };
        let from = if self.host.is_empty() {
            self.from_slug.clone()
        } else {
            format!("{}@{}", self.from_slug, slugify_host(&self.host))
        };
        format!("{from}: {}", truncate_chars(text, MESSAGE_MAX_CHARS))
    }
}

/// Map an ANSI SGR code string to a tmux `#[style]` attribute string.
fn ansi_to_tmux_style(code: &str) -> &'static str {
    match code {
        "36" => "fg=colour6",          // cyan
        "2" => "dim",                  // dim
        "32" => "fg=colour2",          // green
        "1;31" => "fg=colour1,bold",   // bold red
        "1;33" => "fg=colour3,bold",   // bold yellow
        _ => "default",
    }
}

pub fn render_statusline(v: &StatuslineView, color: bool) -> String {
    render_statusline_inner(v, color, false)
}

pub fn render_statusline_tmux(v: &StatuslineView) -> String {
    render_statusline_inner(v, true, true)
}

fn render_statusline_inner(v: &StatuslineView, color: bool, tmux_fmt: bool) -> String {
    let paint = |s: String, code: &str| -> String {
        if tmux_fmt {
            format!("#[{}]{}#[default]", ansi_to_tmux_style(code), s)
        } else if color {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s
        }
    };
    let mut segs: Vec<String> = Vec::new();

    // Identity: who I am on the fabric + which session body I'm wearing.
    segs.push(format!(
        "{}{}",
        paint(
            format!("{}@{}", v.agent, slugify_host(&v.host)),
            "36" // cyan
        ),
        paint(
            format!(" [{}]", SessionId::from(v.session_id.as_str())),
            "2"
        ),
    ));

    // Counts: ⬡ project roster (NIP-29 members), ◉ live sessions (incl. idle).
    // No roster cache (no group management) → no ⬡ segment rather than a lying 0.
    if v.member_count > 0 {
        segs.push(paint(format!("⬡{}", v.member_count), "2"));
    }
    segs.push(paint(format!("◉{}", v.session_count), "2"));

    // Citizenship problem beats cosmetics: surface the membership gap loudly.
    if !v.is_member && v.member_count > 0 {
        segs.push(paint(format!("⚠ not in channel {}", v.project), "1;31"));
    }

    // What this session says it is doing right now (its `who` status).
    if v.working {
        let status = if v.status.is_empty() {
            "working"
        } else {
            &v.status
        };
        segs.push(format!(
            "{} {}",
            paint("✎".to_string(), "32"),
            truncate_chars(status, STATUS_MAX_CHARS)
        ));
    } else {
        segs.push(paint("· idle".to_string(), "2"));
    }

    // Distillation error — flashed in red for up to 5 minutes after the failure.
    if let Some(ref err) = v.distill_error {
        segs.push(paint(
            format!("⚠ distill: {}", truncate_chars(err, 40)),
            "1;31", // bold red
        ));
    }

    // Inbox envelope: a pending mention shows bright; a mention drained in the
    // last 30s lingers dimmed with a ✓ so you see what the agent just consumed.
    if let Some(newest) = v.pending.last() {
        let n = v.pending.len();
        let marker = if n > 1 {
            format!("✉{n}")
        } else {
            "✉".to_string()
        };
        let more = if n > 1 {
            format!(" (+{})", n - 1)
        } else {
            String::new()
        };
        segs.push(paint(
            format!("{marker} {}{more}", newest.preview()),
            "1;33", // bold yellow
        ));
    } else if let Some(newest) = v.recent.last() {
        segs.push(paint(format!("✉✓ {}", newest.preview()), "2"));
    }

    segs.join(" ")
}

/// Char-boundary-safe truncation with an ellipsis.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", cut.trim_end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> StatuslineView {
        StatuslineView {
            agent: "claude".into(),
            host: "Kubrick's Mac".into(),
            session_id: "some-long-uuid".into(),
            project: "tenex-edge".into(),
            member_count: 4,
            session_count: 9,
            is_member: true,
            working: true,
            status: "refactoring inbox envelope".into(),
            pending: vec![],
            recent: vec![],
            distill_error: None,
        }
    }

    fn mention(subject: &str, body: &str) -> MentionView {
        MentionView {
            from_slug: "codex".into(),
            host: "kubrick".into(),
            subject: subject.into(),
            body: body.into(),
        }
    }

    #[test]
    fn renders_identity_counts_and_activity() {
        let s = render_statusline(&view(), false);
        let sid = session_codename("some-long-uuid");
        assert_eq!(
            s,
            format!("claude@kubrick-s-mac [{sid}] ⬡4 ◉9 ✎ refactoring inbox envelope")
        );
    }

    #[test]
    fn renders_pending_mention_with_count() {
        let mut v = view();
        v.pending = vec![mention("", "older"), mention("Please review my work", "x")];
        let s = render_statusline(&v, false);
        assert!(
            s.ends_with("✉2 codex@kubrick: Please review my work (+1)"),
            "got: {s}"
        );
    }

    #[test]
    fn renders_recently_consumed_mention() {
        let mut v = view();
        v.recent = vec![mention("", "Please review my work\nsecond line")];
        let s = render_statusline(&v, false);
        assert!(
            s.ends_with("✉✓ codex@kubrick: Please review my work"),
            "got: {s}"
        );
    }

    #[test]
    fn pending_wins_over_recent() {
        let mut v = view();
        v.pending = vec![mention("new one", "")];
        v.recent = vec![mention("old one", "")];
        let s = render_statusline(&v, false);
        assert!(s.contains("✉ codex@kubrick: new one"), "got: {s}");
        assert!(!s.contains("old one"), "got: {s}");
    }

    #[test]
    fn idle_session_says_idle() {
        let mut v = view();
        v.working = false;
        let s = render_statusline(&v, false);
        assert!(s.ends_with("· idle"), "got: {s}");
    }

    #[test]
    fn membership_gap_is_loud_and_zero_roster_hides_hexagon() {
        let mut v = view();
        v.is_member = false;
        let s = render_statusline(&v, false);
        assert!(s.contains("⚠ not in channel tenex-edge"), "got: {s}");

        v.member_count = 0;
        let s = render_statusline(&v, false);
        assert!(!s.contains('⬡'), "got: {s}");
        assert!(
            !s.contains("not in group"),
            "membership unknown when roster cache is empty: {s}"
        );
    }

    #[test]
    fn truncates_long_status() {
        let mut v = view();
        v.status = "x".repeat(100);
        let s = render_statusline(&v, false);
        assert!(s.contains('…'), "got: {s}");
        assert!(s.chars().count() < 100, "got: {s}");
    }
}
