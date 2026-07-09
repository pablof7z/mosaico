use super::super::*;

/// Set the active publishing channel. When `leave_previous` is true this is the
/// user-facing `channels switch` semantics: leave the previous active channel
/// and join the new one. `channels create` uses `false` so creating a room can
/// move focus into it without dropping the parent from passive context.
pub(in crate::daemon::server) fn set_active_session_channel(
    state: &Arc<DaemonState>,
    session_id: &str,
    agent_pubkey: &str,
    new_channel: &str,
    leave_previous: bool,
) -> Result<()> {
    state.with_store(|s| -> Result<()> {
        // Preflight before mutating; session row and identity channel must move
        // together or not at all.
        let prev_to_leave = if leave_previous {
            s.get_session(session_id)
                .context("set_active_session_channel: reading current session")?
                .map(|r| r.channel_h)
                .filter(|h| h != new_channel)
        } else {
            None
        };
        let mut idn = s
            .identity_for_session(session_id)
            .context("set_active_session_channel: loading identity")?
            .with_context(|| {
                format!(
                    "set_active_session_channel: no identity row for live session \
                     {session_id} (agent {agent_pubkey}); refusing to silently skip the \
                     identity active-channel move"
                )
            })?;
        idn.channel_h = new_channel.to_string();
        idn.session_id = session_id.to_string();
        idn.alive = true;

        if let Some(prev) = prev_to_leave {
            s.leave_session_channel(session_id, &prev)
                .context("set_active_session_channel: leaving previous channel")?;
        }
        s.join_session_channel(session_id, new_channel, now_secs())
            .context("set_active_session_channel: joining new channel")?;
        s.set_session_channel(session_id, new_channel)
            .context("set_active_session_channel: repointing active channel")?;
        s.upsert_identity(&idn)
            .context("set_active_session_channel: persisting identity active-channel move")?;
        Ok(())
    })?;
    state.outbox_notify.notify_waiters();
    Ok(())
}
