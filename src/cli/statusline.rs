//! `tenex-edge statusline` — the fabric, one line at a time.
//!
//! Renders the awareness floor for a host status bar:
//!   claude@kubrick tenex-edge support [Refactoring the inbox] [writing tests]
//!   └ identity ┘  └ project┘  └channel┘ └ distilled title ┘   └ live activity ┘
//!
//! The channel segment is the channel's human NAME (kind:39000 `name`), falling
//! back to its raw id only when no name is cached — the opaque id is never shown
//! when a name exists.
//!
//! `agentName` is exactly what the session published in its kind:0 profile
//! (the `name` field). `host` is the backend's exact config label. `project-name` is
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

pub(super) fn statusline(session: Option<String>, tmux_fmt: bool) -> Result<()> {
    // Harness payload on stdin (absent when invoked by hand from a terminal or
    // from the tmux status-format #(...) invocation).
    let raw: serde_json::Value = if io::stdin().is_terminal() {
        serde_json::Value::Null
    } else {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf).ok();
        serde_json::from_str(&buf).unwrap_or(serde_json::Value::Null)
    };
    // Session ID from stdin payload (Claude Code harness) takes precedence over
    // the explicit --session arg (tmux status-format invocation).
    let session_id = raw
        .get("session_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| session.filter(|s| !s.is_empty()));

    // No session ID from either source — the daemon hasn't stamped @te_session
    // yet (or the format string is wrong). Show a loud error instead of silently
    // querying with null and hiding behind "@unknown".
    let session_id = match session_id {
        Some(id) => id,
        None => {
            let msg = if tmux_fmt {
                "#[fg=colour1,bold][te: @te_session not set — check tenex-edge launch]#[default]"
            } else {
                "[te: no session id — @te_session not set]"
            };
            println!("{msg}");
            return Ok(());
        }
    };

    let params = serde_json::json!({ "session": session_id });
    let v = match crate::daemon::blocking::call_no_spawn("statusline", params) {
        Ok(v) => v,
        Err(_) => {
            // Daemon is not running — emit a visible indicator so the status bar
            // shows WHY it's blank rather than silently displaying nothing.
            println!("[te: down]");
            return Ok(());
        }
    };
    let view = match serde_json::from_value::<StatuslineView>(v) {
        Ok(v) => v,
        Err(e) => {
            let msg = if tmux_fmt {
                format!("#[fg=colour1,bold][te: bad daemon response: {e}]#[default]")
            } else {
                format!("[te: bad daemon response: {e}]")
            };
            println!("{msg}");
            return Ok(());
        }
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
    #[allow(dead_code)]
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
    /// `title` tag). Retained across idle turns and after exit. Rendered as the
    /// `[title]` segment when it differs from the channel name.
    #[serde(default)]
    title: String,
    /// The live "doing now" line from kind:30315 (empty when idle). This is
    /// what `[status]` renders when busy; idle renders `[idle]` instead.
    #[serde(default)]
    activity: String,
    #[serde(default)]
    distill_error: Option<String>,
    /// Populated by the daemon when the session ID is known but can't be
    /// resolved (stale after DB wipe, etc.). Rendered visibly so the user
    /// can see WHY the status bar is broken instead of getting a blank bar.
    #[serde(default)]
    error: Option<String>,
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

    // Identity: the agent's published kind:0 name @ the backend label.
    let ident = if v.host.trim().is_empty() {
        v.agent.clone()
    } else {
        format!("{}@{}", v.agent, v.host.trim())
    };
    segs.push(paint(
        ident, "36", // cyan
    ));

    // Project: the work-root project the session's room hangs under.
    segs.push(paint(v.work_root.clone(), "2"));

    // Session: the channel the session is currently on (its `channel` when set,
    // else its per-session room). Rendered by its human NAME (kind:39000 `name`),
    // falling back to the raw id only when no name is cached — the opaque id is
    // never shown when a name exists. Reflects a `channels switch` immediately.
    let channel_disp = if v.channel_title.trim().is_empty() {
        v.channel.clone()
    } else {
        v.channel_title.clone()
    };
    segs.push(paint(truncate_chars(&channel_disp, TITLE_MAX_CHARS), "2"));

    // Title: the distilled session title (kind:30315), shown while it differs
    // from the channel name. Omitted when empty (brand-new session before any
    // distill) or identical to the channel label already shown.
    if !v.title.trim().is_empty() && v.title != channel_disp {
        segs.push(paint(
            format!("[{}]", truncate_chars(&v.title, TITLE_MAX_CHARS)),
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
            format!("⚠ not in channel {channel_disp}"),
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

    // Daemon-reported error (e.g. stale session ID that wasn't found in the DB).
    // Short and visible — the user needs to know WHY the bar is broken.
    if let Some(ref err) = v.error {
        return paint(format!("[te: {err}]"), "1;31");
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
            // New model: `channel` is the opaque NIP-29 id (never shown when a
            // name is cached), `channel_title` is the channel's human NAME, and
            // `title` is the distilled session title — three distinct values.
            channel: "41yh4c028b76a".into(),
            channel_title: "support".into(),
            member_count: 4,
            is_member: true,
            working: true,
            title: "Refactoring the inbox".into(),
            activity: "writing tests".into(),
            distill_error: None,
            error: None,
        }
    }

    #[test]
    fn renders_identity_project_session_title_status() {
        let s = render_statusline(&view(), false);
        // Channel segment renders the human NAME (`support`), never the opaque
        // id; the distilled session title follows in its own `[…]` segment.
        assert_eq!(
            s,
            "claude@Kubrick's Mac tenex-edge support \
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
        assert!(s.contains("⚠ not in channel support"), "got: {s}");

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
