use super::*;

// ── tail rendering helpers ───────────────────────────────────────────────────

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
#[cfg(test)]
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
            channel,
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
                "{ts_str}  {cat}  {}@{channel}{sess}  {arrow} {}{to_sess}{snippet}",
                col!(from, cyan),
                col!(to, cyan),
            )
        }

        TailEvent::Sync {
            from,
            to,
            state,
            detail,
            ..
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
            let detail = if compact {
                String::new()
            } else {
                detail
                    .as_deref()
                    .filter(|d| !d.trim().is_empty())
                    .map(|d| format!(" - {}", d.replace('\n', " ")))
                    .unwrap_or_default()
            };
            format!("{ts_str}  {cat_str}  {from} → {to}  {glyph} {state}{detail}")
        }

        TailEvent::Turn {
            channel,
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
                "{ts_str}  {cat}  {}@{channel}{sess}  {glyph}{detail}",
                col!(agent, cyan),
            )
        }

        TailEvent::Status {
            channel,
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
            format!("{ts_str}  {cat}  {}@{channel}  {label}", col!(agent, cyan))
        }

        TailEvent::Join {
            channel,
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
                "{ts_str}  {cat}  {}@{host}{sess}  online ({channel}{cwd_info})",
                col!(agent, cyan),
            )
        }

        TailEvent::Leave {
            channel,
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
                "{ts_str}  {cat}  {}@{host}{sess}  offline (was online {dur}, {channel})",
                col!(agent, cyan),
            )
        }

        TailEvent::Sess {
            channel,
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
                "{ts_str}  {cat}  {}@{channel}{sess}  session {state}{cwd_info}",
                col!(agent, cyan),
            )
        }

        TailEvent::Proj { channel, about, .. } => {
            let cat = col!("proj ", dimmed);
            let snippet: String = about.chars().take(60).collect();
            format!("{ts_str}  {cat}  {channel}  {snippet}")
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

#[cfg(test)]
fn fmt_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}
