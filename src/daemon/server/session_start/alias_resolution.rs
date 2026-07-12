use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_session_id(
    state: &Arc<DaemonState>,
    harness: &str,
    pty_session: Option<&str>,
    harness_session_id: Option<&str>,
    resume_id: Option<&str>,
    watch_pid: Option<i32>,
    durable_agent: bool,
    now: u64,
) -> Result<(String, &'static str, String)> {
    // Canonical identity: the daemon MINTS a stable session id; the harness id /
    // resume token / endpoint / pid become rows in `session_aliases`. For
    // PTY-hosted launches, a delayed harness hook must reassert the spawn-time
    // PTY registration instead of minting a second row under its native id.
    let existing_pty_session = pty_session.and_then(|pty| {
        state
            .with_store(|s| s.alive_session_for_alias(None, "pty_session", pty))
            .ok()
            .flatten()
            .map(|rec| rec.session_id)
    });

    let (ext_kind, ext_id) = match (
        existing_pty_session.as_ref(),
        pty_session,
        harness_session_id,
        resume_id,
        watch_pid,
    ) {
        (Some(_), Some(pty), _, _, _) => ("pty_session", pty.to_string()),
        (None, _, Some(hs), _, _) => ("harness_session", hs.to_string()),
        (None, _, None, Some(resume), _) => ("resume", resume.to_string()),
        (None, _, None, None, Some(pid)) => ("watch_pid", pid.to_string()),
        (None, Some(pty), None, None, None) => ("pty_session", pty.to_string()),
        _ => ("harness_session", String::new()),
    };

    let session_id = if let Some(session_id) = existing_pty_session {
        state.with_store(|s| s.put_alias(harness, ext_kind, &ext_id, &session_id, now))?;
        session_id
    } else if durable_agent {
        state.with_store(|s| s.resolve_live_or_mint_session_id(harness, ext_kind, &ext_id, now))?
    } else {
        state.with_store(|s| s.resolve_or_mint_session_id(harness, ext_kind, &ext_id, now))?
    };
    Ok((session_id, ext_kind, ext_id))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn record_secondary_aliases(
    state: &Arc<DaemonState>,
    harness: &str,
    session_id: &str,
    pty_session: Option<&str>,
    pty_socket: Option<&str>,
    harness_session_id: Option<&str>,
    resume_id: Option<&str>,
    watch_pid: Option<i32>,
    work_root: &str,
    cwd: &std::path::Path,
    channel: &str,
    now: u64,
) {
    state.with_store(|s| {
        for (kind, value) in [
            ("pty_session", pty_session),
            ("pty_socket", pty_socket),
            ("harness_session", harness_session_id),
            ("resume", resume_id),
        ] {
            if let Some(value) = value {
                s.put_alias(harness, kind, value, session_id, now).ok();
            }
        }
        if let Some(pid) = watch_pid {
            s.put_alias(harness, "watch_pid", &pid.to_string(), session_id, now)
                .ok();
        }
        s.upsert_workspace(work_root, &cwd.to_string_lossy(), now)
            .ok();
        if channel != work_root {
            s.upsert_workspace(channel, &cwd.to_string_lossy(), now)
                .ok();
        }
    });
}
