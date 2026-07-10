//! `who --expired` renderer: the dead/old sessions a user can resume.

use crate::expired_sessions::ExpiredSessionRow;
use owo_colors::OwoColorize as _;
use std::fmt::Write as _;

/// Render the expired-session listing: each session as `@agent/codename` with
/// its channel, last-seen age, and whether it can be resumed. Newest first (as
/// returned by the daemon).
pub(in crate::cli::who) fn render_expired(rows: &[ExpiredSessionRow]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{}", "Expired sessions".bold());
    let _ = writeln!(out);
    if rows.is_empty() {
        let _ = writeln!(out, "(no expired sessions)");
        return out;
    }
    let now = crate::util::now_secs();
    for row in rows {
        let handle = format!(
            "@{}",
            crate::idref::session_handle(&row.agent_slug, &row.codename)
        );
        let seen = crate::util::relative_time(row.last_seen, now);
        let resumable = if row.resumable {
            "resumable".green().to_string()
        } else {
            "not resumable".dimmed().to_string()
        };
        let _ = writeln!(
            out,
            "{}  #{}  seen {}  {}",
            handle.cyan(),
            row.channel,
            seen.dimmed(),
            resumable,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(codename: &str, resumable: bool) -> ExpiredSessionRow {
        ExpiredSessionRow {
            agent_slug: "coder".into(),
            session_id: "sess-abc".into(),
            codename: codename.into(),
            host: "laptop".into(),
            channel: "main".into(),
            last_seen: 0,
            resumable,
        }
    }

    #[test]
    fn renders_agent_session_handle_and_resumability() {
        let out = render_expired(&[row("amber-echo-001", true), row("cedar-mesa-002", false)]);
        assert!(out.contains("@coder/amber-echo-001"), "got: {out}");
        assert!(out.contains("@coder/cedar-mesa-002"), "got: {out}");
        assert!(out.contains("#main"), "got: {out}");
        assert!(out.contains("resumable"), "got: {out}");
        assert!(out.contains("not resumable"), "got: {out}");
    }

    #[test]
    fn empty_listing_is_explained() {
        let out = render_expired(&[]);
        assert!(out.contains("no expired sessions"), "got: {out}");
    }
}
