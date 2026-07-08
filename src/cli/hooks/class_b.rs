const IDLE_NUDGE: &str = "[tenex-edge] Are you done with this spawned session? If so, run `tenex-edge session end --self`; otherwise continue.";

pub(super) fn inject_idle_nudge(agent_slug: &str) {
    let pty_session = crate::cli::pty_session_env();
    if !should_nudge(
        crate::cli::ephemeral_session_env(),
        pty_session.as_deref(),
        agent_slug,
    ) {
        return;
    }
    let Some(pty_id) = pty_session else {
        return;
    };
    if !crate::pty::is_live(&pty_id) {
        return;
    }
    if let Err(e) = crate::pty::inject(&pty_id, IDLE_NUDGE, true, true) {
        eprintln!("[tenex-edge] failed to inject Class B idle nudge: {e:#}");
    }
}

fn should_nudge(ephemeral: bool, pty_session: Option<&str>, agent_slug: &str) -> bool {
    ephemeral
        && pty_session.is_some_and(|id| !id.is_empty())
        && !crate::session_host::agent_supports_headless_exec(agent_slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nudge_requires_ephemeral_pty_and_non_headless_agent() {
        // grok has a resume shape but no headless exec, so it stays Class B (PTY
        // + idle nudge). claude/codex/opencode run headless and are never nudged.
        assert!(should_nudge(true, Some("pty-1"), "grok"));
        assert!(!should_nudge(false, Some("pty-1"), "grok"));
        assert!(!should_nudge(true, None, "grok"));
        assert!(!should_nudge(true, Some("pty-1"), "codex"));
        assert!(!should_nudge(true, Some("pty-1"), "opencode"));
    }

    #[test]
    fn nudge_names_self_end_command() {
        assert!(IDLE_NUDGE.contains("tenex-edge session end --self"));
    }
}
