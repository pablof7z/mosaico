use super::super::*;

/// Repoint the session's active publishing channel, leaving the previous one
/// joined as passive context. The caller must already hold a route to
/// `new_channel`; this only moves focus, it never joins or leaves.
pub(in crate::daemon::server) fn set_active_session_channel(
    state: &Arc<DaemonState>,
    pubkey: &str,
    new_channel: &str,
) -> Result<()> {
    state.with_store(|s| -> Result<()> {
        let current = s
            .get_session(pubkey)
            .context("set_active_session_channel: reading current session")?
            .with_context(|| format!("set_active_session_channel: no live session for {pubkey}"))?;
        if !current.is_running()
            || current.recovery_state == crate::state::RecoveryState::Revoked
            || !s.has_session_route(pubkey, new_channel)?
        {
            anyhow::bail!("set_active_session_channel: session lifecycle changed");
        }
        s.set_session_channel(pubkey, new_channel)
            .context("set_active_session_channel: repointing active channel")?;
        Ok(())
    })?;
    Ok(())
}
