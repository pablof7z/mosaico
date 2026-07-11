//! `who --expired` renderer: the dead/old sessions a user can resume.

use crate::expired_sessions::ExpiredSessionRow;
use owo_colors::OwoColorize as _;
use std::fmt::Write as _;

/// Render expired sessions with npub as the durable copy-paste selector.
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
        let identity = match row.handle.as_deref() {
            Some(handle) => format!("@{handle}  {}", row.npub),
            None => row.npub.clone(),
        };
        let seen = crate::util::relative_time(row.last_seen, now);
        let resumable = if row.resumable {
            "resumable".green().to_string()
        } else {
            "not resumable".dimmed().to_string()
        };
        let _ = writeln!(
            out,
            "{}  #{}  seen {}  {}",
            identity.cyan(),
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

    fn row(handle: Option<&str>, resumable: bool) -> ExpiredSessionRow {
        ExpiredSessionRow {
            agent_slug: "coder".into(),
            pubkey: "11".repeat(32),
            npub: "npub1durable".into(),
            handle: handle.map(str::to_string),
            host: "laptop".into(),
            channel: "main".into(),
            last_seen: 0,
            resumable,
        }
    }

    #[test]
    fn renders_agent_session_handle_and_resumability() {
        let out = render_expired(&[row(Some("amber-coder"), true), row(None, false)]);
        assert!(out.contains("@amber-coder"), "got: {out}");
        assert!(out.contains("npub1durable"), "got: {out}");
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
