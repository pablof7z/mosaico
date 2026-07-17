const IDLE_NUDGE: &str = "[mosaico] Are you done with this spawned session? If so, run `mosaico my session end --self`; otherwise continue.";

pub(super) fn inject_idle_nudge() {
    let pty_session = crate::cli::pty_session_env();
    if !should_nudge(crate::cli::ephemeral_session_env(), pty_session.as_deref()) {
        return;
    }
    let Some(pty_id) = pty_session else {
        return;
    };
    if !crate::pty::is_live(&pty_id) {
        return;
    }
    if let Err(e) = crate::pty::inject(&pty_id, IDLE_NUDGE, true, true) {
        eprintln!("[mosaico] failed to inject Class B idle nudge: {e:#}");
    }
}

fn should_nudge(ephemeral: bool, pty_session: Option<&str>) -> bool {
    ephemeral && pty_session.is_some_and(|id| !id.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nudge_requires_an_ephemeral_pty() {
        assert!(should_nudge(true, Some("pty-1")));
        assert!(!should_nudge(false, Some("pty-1")));
        assert!(!should_nudge(true, None));
    }

    #[test]
    fn nudge_names_self_end_command() {
        assert!(IDLE_NUDGE.contains("mosaico my session end --self"));
    }
}
