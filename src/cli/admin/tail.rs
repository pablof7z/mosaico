#![allow(dead_code)]

use super::*;
use std::time::Duration;

// ── tail ─────────────────────────────────────────────────────────────────────

/// Options for the `tail` command.
pub struct TailOpts {
    pub project: Option<String>,
    pub agent: Option<String>,
    pub host: Option<String>,
    pub since: Option<String>,
    pub backfill: Option<u64>,
    pub only: Option<String>,
    pub exclude: Option<String>,
    pub include: Option<String>,
    pub all: bool,
    pub compact: bool,
    pub relative: bool,
    pub no_emoji: bool,
    pub no_color: bool,
    pub json: bool,
    pub no_follow: bool,
    pub live: bool,
}

pub async fn tail(opts: TailOpts) -> Result<()> {
    if opts.live {
        eprintln!(
            "tenex-edge tail --live: the full-screen TUI dashboard is not yet implemented. \
             Use bare `tenex-edge tail` for the live scrolling feed."
        );
        return Ok(());
    }

    // Resolve color + emoji settings: explicit flags override env/TTY.
    let use_color =
        !opts.no_color && std::env::var("NO_COLOR").is_err() && std::io::stdout().is_terminal();
    let use_emoji = !opts.no_emoji;

    // Parse --since into a unix timestamp.
    let since_ts: u64 = opts.since.as_deref().map(parse_since).unwrap_or(0);

    let scope_label = opts.project.as_deref().unwrap_or("*");
    if !opts.json {
        eprintln!(
            "{} tailing project {} … (Ctrl-C to stop)",
            if use_color {
                "tenex-edge".bold().to_string()
            } else {
                "tenex-edge".to_string()
            },
            if use_color {
                scope_label.cyan().to_string()
            } else {
                scope_label.to_string()
            },
        );
    }

    // Build the category filter set.
    let cats_only: Option<std::collections::HashSet<String>> = opts
        .only
        .as_deref()
        .map(|s| s.split(',').map(|c| c.trim().to_lowercase()).collect());
    let cats_exclude: std::collections::HashSet<String> = opts
        .exclude
        .as_deref()
        .map(|s| s.split(',').map(|c| c.trim().to_lowercase()).collect())
        .unwrap_or_default();
    let cats_include: std::collections::HashSet<String> = opts
        .include
        .as_deref()
        .map(|s| s.split(',').map(|c| c.trim().to_lowercase()).collect())
        .unwrap_or_default();

    // Minimum tier: default hides tier 0 (profile); --all includes all; --v same.
    let min_tier: u8 = if opts.all { 0 } else { 1 };

    let params = serde_json::json!({
        "project": opts.project,
        "backfill": opts.backfill.unwrap_or(20),
        "since": since_ts,
    });

    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;

    let agent_filter = opts.agent.clone();
    let host_filter = opts.host.clone();
    let is_json = opts.json;
    let no_follow = opts.no_follow;
    let compact = opts.compact;
    let relative = opts.relative;

    let stream = client.stream("tail", params, move |item| {
        // Deserialize TailEvent.
        let ev: crate::daemon::tail_event::TailEvent = match serde_json::from_value(item.clone()) {
            Ok(e) => e,
            Err(_) => {
                // Fallback: if we get an old {line} format, print it.
                if let Some(line) = item.get("line").and_then(|l| l.as_str()) {
                    println!("{line}");
                }
                return;
            }
        };

        // Apply agent/host filters.
        if let Some(ref ag) = agent_filter {
            let ev_agent = match &ev {
                crate::daemon::tail_event::TailEvent::Msg { from, .. } => from.as_str(),
                crate::daemon::tail_event::TailEvent::Turn { agent, .. } => agent.as_str(),
                crate::daemon::tail_event::TailEvent::Status { agent, .. } => agent.as_str(),
                crate::daemon::tail_event::TailEvent::Join { agent, .. } => agent.as_str(),
                crate::daemon::tail_event::TailEvent::Leave { agent, .. } => agent.as_str(),
                crate::daemon::tail_event::TailEvent::Sess { agent, .. } => agent.as_str(),
                crate::daemon::tail_event::TailEvent::Profile { agent, .. } => agent.as_str(),
                _ => "",
            };
            if !ev_agent.is_empty() && ev_agent != ag.as_str() {
                return;
            }
        }
        if let Some(ref h) = host_filter {
            let ev_host = match &ev {
                crate::daemon::tail_event::TailEvent::Join { host, .. } => host.as_str(),
                crate::daemon::tail_event::TailEvent::Leave { host, .. } => host.as_str(),
                crate::daemon::tail_event::TailEvent::Profile { host, .. } => host.as_str(),
                _ => "",
            };
            if !ev_host.is_empty() && ev_host != h.as_str() {
                return;
            }
        }

        // Tier filter.
        if ev.tier() < min_tier && !cats_include.contains(ev.category()) {
            return;
        }

        // Category filters.
        let cat = ev.category();
        if let Some(ref only) = cats_only {
            if !only.contains(cat) {
                return;
            }
        }
        if cats_exclude.contains(cat) && !cats_include.contains(cat) {
            return;
        }

        // Render.
        if is_json {
            if let Ok(s) = serde_json::to_string(&ev) {
                println!("{s}");
            }
        } else {
            let line = render_tail_event(&ev, use_color, use_emoji, relative, compact);
            println!("{line}");
        }
    });

    if no_follow {
        // For no-follow: run with a short timeout to get just the backfill.
        // The daemon will keep streaming; we disconnect after receiving the
        // initial batch. Since we can't easily detect "backfill done", we
        // use a small sleep approach: connect, get backfill, disconnect.
        tokio::select! {
            r = stream => r,
            _ = tokio::time::sleep(Duration::from_millis(500)) => Ok(()),
        }
    } else {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => Ok(()),
            r = stream => r,
        }
    }
}

/// Parse a --since value into a unix timestamp.
/// Accepts: unix seconds ("1700000000"), or durations ("1h", "30m", "2d").
pub fn parse_since(s: &str) -> u64 {
    let now = now_secs();
    if let Ok(ts) = s.parse::<u64>() {
        return ts;
    }
    // Simple duration parsing: Nh, Nm, Nd, Ns.
    let s = s.trim();
    let (num_str, unit) = s.split_at(s.len().saturating_sub(1));
    if let Ok(n) = num_str.trim().parse::<u64>() {
        let secs = match unit {
            "h" | "H" => n * 3600,
            "m" | "M" => n * 60,
            "d" | "D" => n * 86400,
            "s" | "S" => n,
            _ => n,
        };
        return now.saturating_sub(secs);
    }
    0
}

/// Render a `TailEvent` to a human-readable string.
///
/// `use_color` and `use_emoji` are passed explicitly so this fn is testable
/// without side-effects from TTY detection or NO_COLOR.
pub fn render_tail_event(
    ev: &crate::daemon::tail_event::TailEvent,
    use_color: bool,
    use_emoji: bool,
    relative: bool,
    compact: bool,
) -> String {
    use crate::daemon::tail_event::TailEvent;

    let ts = ev.ts();
    let ts_str = if relative {
        let age = now_secs().saturating_sub(ts);
        if age < 60 {
            format!("{age}s ago")
        } else if age < 3600 {
            format!("{}m ago", age / 60)
        } else {
            format!("{}h ago", age / 3600)
        }
    } else {
        // Wall-clock HH:MM:SS.
        let h = (ts % 86400) / 3600;
        let m = (ts % 3600) / 60;
        let s = ts % 60;
        format!("{h:02}:{m:02}:{s:02}")
    };

    // Helper: colorize if color enabled.
    macro_rules! col {
        ($text:expr, $color:ident) => {
            if use_color {
                $text.$color().to_string()
            } else {
                $text.to_string()
            }
        };
    }

    // Short raw-session-id correlation handle. Operator-facing handle to correlate
    // lines for the same session; identity is the agent label.
    let sess_code = |sid: &str| sid.chars().take(8).collect::<String>();

    match ev {
        TailEvent::Msg {
            project,
            from,
            from_session,
            to,
            to_session,
            body,
            ..
        } => {
            let cat = col!("msg  ", yellow);
            let arrow = if use_emoji { "→" } else { "->" };
            let sess = from_session
                .as_deref()
                .map(|s| format!("[{}]", sess_code(s)))
                .unwrap_or_default();
            let to_sess = to_session
                .as_deref()
                .map(|s| format!("[{}]", sess_code(s)))
                .unwrap_or_default();
            let snippet = if compact {
                String::new()
            } else {
                let body_clean: String = body.chars().take(72).collect();
                let body_clean = body_clean.replace('\n', " ");
                let ellipsis = if body.len() > 72 { "…" } else { "" };
                format!(" \"{}{}\"", body_clean, ellipsis)
            };
            format!(
                "{ts_str}  {cat}  {}@{project}{sess}  {arrow} {}{to_sess}{snippet}",
                col!(from, cyan),
                col!(to, cyan),
            )
        }

        TailEvent::Sync {
            from, to, state, ..
        } => {
            let (cat, color_fn): (&str, fn(&str) -> String) = match state.as_str() {
                "failed" => ("sync ", |s| {
                    if true {
                        s.red().to_string()
                    } else {
                        s.to_string()
                    }
                }),
                _ => ("sync ", |s| s.cyan().to_string()),
            };
            let cat_str = if use_color {
                match state.as_str() {
                    "failed" => col!(cat, red),
                    _ => col!(cat, cyan),
                }
            } else {
                cat.to_string()
            };
            let _ = color_fn; // suppress unused warning
            let glyph = if use_emoji {
                match state.as_str() {
                    "delivered" => "✓",
                    "failed" => "✗",
                    _ => "~",
                }
            } else {
                match state.as_str() {
                    "delivered" => "[ok]",
                    "failed" => "[x]",
                    _ => "~",
                }
            };
            format!("{ts_str}  {cat_str}  {from} → {to}  {glyph} {state}")
        }

        TailEvent::Turn {
            project,
            agent,
            session,
            state,
            elapsed_s,
            ..
        } => {
            let cat = col!("turn ", green);
            let sess = format!("[{}]", sess_code(session));
            let (glyph, detail) = if state == "working" {
                let g = if use_emoji { "▶" } else { ">" };
                (g, " started working".to_string())
            } else {
                let g = if use_emoji { "⏸" } else { "||" };
                let dur = elapsed_s
                    .map(|e| format!(" ({})", fmt_duration(e)))
                    .unwrap_or_default();
                (g, format!(" idle{dur}"))
            };
            format!(
                "{ts_str}  {cat}  {}@{project}{sess}  {glyph}{detail}",
                col!(agent, cyan),
            )
        }

        TailEvent::Status {
            project,
            agent,
            text,
            active,
            ..
        } => {
            let cat = col!("stat ", magenta);
            let label = match (text.is_empty(), *active) {
                (true, true) => "working".to_string(),
                (true, false) => "idle".to_string(),
                (false, true) => text.clone(),
                (false, false) => format!("{text} · idle"),
            };
            format!("{ts_str}  {cat}  {}@{project}  {label}", col!(agent, cyan))
        }

        TailEvent::Join {
            project,
            agent,
            host,
            session,
            rel_cwd,
            ..
        } => {
            let cat = col!("join ", green);
            let sess = format!("[{}]", sess_code(session));
            let cwd_info = if rel_cwd.is_empty() || rel_cwd == "." {
                String::new()
            } else {
                format!(" ({})", rel_cwd)
            };
            format!(
                "{ts_str}  {cat}  {}@{host}{sess}  online ({project}{cwd_info})",
                col!(agent, cyan),
            )
        }

        TailEvent::Leave {
            project,
            agent,
            host,
            session,
            online_s,
            ..
        } => {
            let cat = col!("leave", dimmed);
            let sess = format!("[{}]", sess_code(session));
            let dur = fmt_duration(*online_s);
            format!(
                "{ts_str}  {cat}  {}@{host}{sess}  offline (was online {dur}, {project})",
                col!(agent, cyan),
            )
        }

        TailEvent::Sess {
            project,
            agent,
            session,
            state,
            rel_cwd,
            ..
        } => {
            let cat = col!("sess ", blue);
            let sess = format!("[{}]", sess_code(session));
            let cwd_info = if rel_cwd.is_empty() || rel_cwd == "." {
                String::new()
            } else {
                format!(" (rel_cwd: {rel_cwd})")
            };
            format!(
                "{ts_str}  {cat}  {}@{project}{sess}  session {state}{cwd_info}",
                col!(agent, cyan),
            )
        }

        TailEvent::Proj { project, about, .. } => {
            let cat = col!("proj ", dimmed);
            let snippet: String = about.chars().take(60).collect();
            format!("{ts_str}  {cat}  {project}  {snippet}")
        }

        TailEvent::Profile {
            agent,
            host,
            pubkey,
            ..
        } => {
            let cat = col!("id   ", dimmed);
            let pk_short = &pubkey[..pubkey.len().min(8)];
            format!(
                "{ts_str}  {cat}  {}@{host}  ({pk_short})",
                col!(agent, cyan)
            )
        }
    }
}

fn fmt_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}
