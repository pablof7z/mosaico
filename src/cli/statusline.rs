//! `tenex-edge statusline` — the fabric, one line at a time.
//!
//! Renders the awareness floor for a host status bar:
//!   claude@kubrick tenex-edge #bravo4217 [Refactoring the inbox] [writing tests]
//!   └ identity ┘  └ project┘  └ session ┘  └ channel title┘   └ live activity ┘
//!
//! `agentName` is exactly what the session published in its kind:0 profile
//! (the `name` field). `host` is the slugified machine host. `project-name` is
//! the work-root project the session room hangs under. `#session` is the
//! channel the session is currently on (changes with `tenex-edge channels
//! switch`). `[title]` is that channel's title on the relay (kind:39000 `name`
//! tag for a task channel; the distilled session title for a per-session
//! room). `[status]` is what the agent last published in its kind:30315 — the
//! live activity line when busy, or `idle` when not.
//!
//! Optional warning segments (kept per user request) append after `[status]`:
//!   - `⚠ not in channel <name>` — citizenship problem (not a member of the
//!     channel's NIP-29 group).
//!   - `⚠ distill: <message>` — distillation failure flash (up to 5 min).
//!
//! Reads the harness's statusline JSON payload on stdin (Claude Code sends
//! `session_id` + `workspace.current_dir`), asks the daemon for one pure-read
//! snapshot, prints one line. Harnesses re-run this constantly, so it must
//! fail open — daemon down → print nothing, exit 0, and NEVER spawn a daemon
//! just to draw a line.

use super::*;

/// Cap for the channel title and live-activity segments.
const TITLE_MAX_CHARS: usize = 48;
const ACTIVITY_MAX_CHARS: usize = 48;

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
        "group": crate::cli::channel_env(),
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
    /// The agent's published name — exactly the `name` field of its kind:0
    /// profile (the durable identity on the fabric). Renamed from `agent` to
    /// make the kind:0 correspondence explicit.
    #[serde(default)]
    agent: String,
    #[serde(default)]
    host: String,
    #[serde(default)]
    session_id: String,
    /// The work-root project the session's room hangs under (== `who`'s
    /// "Project:" line). For an ordinary project session this is `project`
    /// itself; for a per-session room it's the parent project.
    #[serde(default)]
    work_root: String,
    /// The NIP-29 channel the session is currently routing under — its
    /// `channel` when set (via `tenex-edge channels switch`), else its
    /// per-session room `project`. The `#session-…` segment renders this id.
    #[serde(default)]
    channel: String,
    /// The channel's display title on the relay (kind:39000 `name` tag for a
    /// task channel; the distilled session title for a per-session room).
    /// Falls back to the distilled session title when the local metadata
    /// cache lags. Empty only when neither is known (brand-new session).
    #[serde(default)]
    channel_title: String,
    #[serde(default)]
    member_count: u64,
    #[serde(default = "default_true")]
    is_member: bool,
    #[serde(default)]
    working: bool,
    /// The persistent distilled session title (carried on kind:30315 as the
    /// `title` tag). Retained across idle turns and after exit. Surfaced
    /// indirectly via `channel_title` for a per-session room; kept here for
    /// the fallback when the channel has no relay-echoed name yet.
    #[serde(default)]
    title: String,
    /// The live "doing now" line from kind:30315 (empty when idle). This is
    /// what `[status]` renders when busy; idle renders `[idle]` instead.
    #[serde(default)]
    activity: String,
    #[serde(default)]
    distill_error: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Map an ANSI SGR code string to a tmux `#[style]` attribute string.
fn ansi_to_tmux_style(code: &str) -> &'static str {
    match code {
        "36" => "fg=colour6",        // cyan
        "2" => "dim",                // dim
        "32" => "fg=colour2",        // green
        "1;31" => "fg=colour1,bold", // bold red
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

    // Identity: the agent's published kind:0 name @ the slugified host.
    segs.push(paint(
        format!("{}@{}", v.agent, slugify_host(&v.host)),
        "36", // cyan
    ));

    // Project: the work-root project the session's room hangs under.
    segs.push(paint(v.work_root.clone(), "2"));

    // Session: the channel the session is currently on (its `channel` when
    // set, else its per-session room). Rendered as `#<channel-id>` so a
    // `channels switch` is reflected immediately — matches what the relay
    // shows as the room's `h` tag.
    segs.push(paint(format!("#{}", v.channel), "2"));

    // Title: the channel's title on the relay (== the channel's display name
    // from kind:39000, == the distilled session title for a per-session room).
    // Omitted when empty (brand-new session before any distill).
    if !v.channel_title.is_empty() {
        segs.push(paint(
            format!("[{}]", truncate_chars(&v.channel_title, TITLE_MAX_CHARS)),
            "2",
        ));
    }

    // Status: what the agent last published in its kind:30315. The live
    // activity line when busy; `idle` when not. A busy session with no live
    // activity line shows `working` (matches `who`'s status_plain).
    let status = if v.working {
        if v.activity.is_empty() {
            "working".to_string()
        } else {
            truncate_chars(&v.activity, ACTIVITY_MAX_CHARS)
        }
    } else {
        "idle".to_string()
    };
    segs.push(paint(format!("[{status}]"), "2"));

    // ── Optional warning segments (kept per user request) ──────────────────

    // Citizenship problem beats cosmetics: surface the membership gap loudly.
    // Only when the roster is non-empty (otherwise unknown, not a problem).
    if !v.is_member && v.member_count > 0 {
        segs.push(paint(
            format!("⚠ not in channel {}", v.channel),
            "1;31", // bold red
        ));
    }

    // Distillation error — flashed in red for up to 5 minutes after the failure.
    if let Some(ref err) = v.distill_error {
        segs.push(paint(
            format!("⚠ distill: {}", truncate_chars(err, 40)),
            "1;31", // bold red
        ));
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
            work_root: "tenex-edge".into(),
            channel: "session-a1b2c3d4e5f60718".into(),
            channel_title: "Refactoring the inbox".into(),
            member_count: 4,
            is_member: true,
            working: true,
            title: "Refactoring the inbox".into(),
            activity: "writing tests".into(),
            distill_error: None,
        }
    }

    #[test]
    fn renders_identity_project_session_title_status() {
        let s = render_statusline(&view(), false);
        assert_eq!(
            s,
            "claude@kubrick-s-mac tenex-edge #session-a1b2c3d4e5f60718 \
             [Refactoring the inbox] [writing tests]"
        );
    }

    #[test]
    fn busy_with_no_activity_shows_working() {
        let mut v = view();
        v.activity = String::new();
        let s = render_statusline(&v, false);
        assert!(s.ends_with("[working]"), "got: {s}");
    }

    #[test]
    fn idle_shows_idle() {
        let mut v = view();
        v.working = false;
        let s = render_statusline(&v, false);
        assert!(s.ends_with("[idle]"), "got: {s}");
    }

    #[test]
    fn empty_channel_title_omits_title_segment() {
        let mut v = view();
        v.channel_title = String::new();
        let s = render_statusline(&v, false);
        assert!(!s.contains("[]"), "empty title segment rendered: {s}");
        // Status segment still present.
        assert!(s.contains("[writing tests]"), "got: {s}");
    }

    #[test]
    fn membership_gap_is_loud() {
        let mut v = view();
        v.is_member = false;
        let s = render_statusline(&v, false);
        assert!(
            s.contains("⚠ not in channel session-a1b2c3d4e5f60718"),
            "got: {s}"
        );

        // Unknown roster (count 0) → no warning (unknown, not a problem).
        v.member_count = 0;
        let s = render_statusline(&v, false);
        assert!(!s.contains("not in channel"), "got: {s}");
    }

    #[test]
    fn distill_error_flashes_red() {
        let mut v = view();
        v.distill_error = Some("LLM rate-limited".into());
        let s = render_statusline(&v, false);
        assert!(s.contains("⚠ distill: LLM rate-limited"), "got: {s}");
    }

    #[test]
    fn truncates_long_channel_title() {
        let mut v = view();
        v.channel_title = "x".repeat(100);
        let s = render_statusline(&v, false);
        assert!(s.contains('…'), "got: {s}");
    }

    #[test]
    fn truncates_long_activity() {
        let mut v = view();
        v.activity = "y".repeat(100);
        let s = render_statusline(&v, false);
        assert!(s.contains('…'), "got: {s}");
    }
}
