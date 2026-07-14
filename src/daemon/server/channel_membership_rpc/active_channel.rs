use super::super::*;

/// Set the active publishing channel. When `leave_previous` is true this is the
/// user-facing `channel switch` semantics: leave the previous active channel
/// and join the new one. `channel create` uses `false` so creating a room can
/// move focus into it without dropping the parent from passive context.
pub(in crate::daemon::server) fn set_active_session_channel(
    state: &Arc<DaemonState>,
    pubkey: &str,
    new_channel: &str,
    leave_previous: bool,
) -> Result<()> {
    state.with_store(|s| -> Result<()> {
        let current = s
            .get_session(pubkey)
            .context("set_active_session_channel: reading current session")?
            .with_context(|| format!("set_active_session_channel: no live session for {pubkey}"))?;
        let prev_to_leave = if leave_previous && current.channel_h != new_channel {
            Some(current.channel_h)
        } else {
            None
        };

        if let Some(prev) = prev_to_leave {
            s.leave_session_channel(pubkey, &prev)
                .context("set_active_session_channel: leaving previous channel")?;
        }
        s.set_session_channel(pubkey, new_channel)
            .context("set_active_session_channel: repointing active channel")?;
        Ok(())
    })?;
    state.outbox_notify.notify_waiters();
    Ok(())
}
