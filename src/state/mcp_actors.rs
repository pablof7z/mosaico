//! Durable correlation between redacted remote MCP callers and local sessions.

use super::*;

impl Store {
    pub(crate) fn is_mcp_actor_pubkey(&self, pubkey: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM mcp_actor_aliases WHERE pubkey=?1)",
            [pubkey],
            |row| row.get(0),
        )?)
    }

    pub(crate) fn mcp_actor_pubkey(&self, actor_key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT pubkey FROM mcp_actor_aliases WHERE actor_key=?1",
                [actor_key],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub(crate) fn bind_mcp_actor(
        &self,
        actor_key: &str,
        actor_kind: &str,
        pubkey: &str,
        now: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO mcp_actor_aliases
                 (actor_key, actor_kind, pubkey, created_at, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(actor_key) DO UPDATE SET
                 last_seen=excluded.last_seen",
            params![actor_key, actor_kind, pubkey, now],
        )?;
        let stored: (String, String) = self.conn.query_row(
            "SELECT actor_kind, pubkey FROM mcp_actor_aliases WHERE actor_key=?1",
            [actor_key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        anyhow::ensure!(
            stored == (actor_kind.to_string(), pubkey.to_string()),
            "MCP actor identity changed for {actor_key}"
        );
        Ok(())
    }

    pub(crate) fn reserve_mcp_actor_session(
        &self,
        pubkey: &str,
        agent_slug: &str,
        channel: &str,
        now: u64,
    ) -> Result<()> {
        let idle_deadline = now.saturating_add(HEADLESS_IDLE_TIMEOUT_SECS);
        self.conn.execute(
            "INSERT INTO sessions
                 (pubkey, runtime_generation, agent_slug, channel_h, work_root,
                  runtime_state, presentation_state, work_state, recovery_state,
                  lifecycle_epoch, idle_since, idle_deadline, created_at, last_seen)
             VALUES (?1, 1, ?2, ?3, ?3, 'running', 'headless', 'idle', 'ready',
                     1, ?4, ?5, ?4, ?4)",
            params![pubkey, agent_slug, channel, now, idle_deadline],
        )?;
        self.grant_session_route(pubkey, channel, now)?;
        Ok(())
    }

    pub(crate) fn activate_mcp_actor(&self, actor_key: &str, pubkey: &str, now: u64) -> Result<()> {
        let idle_deadline = now.saturating_add(HEADLESS_IDLE_TIMEOUT_SECS);
        let (state, recovery): (String, String) = self.conn.query_row(
            "SELECT runtime_state, recovery_state FROM sessions WHERE pubkey=?1",
            [pubkey],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        anyhow::ensure!(
            state != "stopping",
            "MCP actor is currently stopping; retry the call"
        );
        anyhow::ensure!(recovery != "revoked", "MCP actor recovery is revoked");
        self.conn.execute(
            "UPDATE sessions SET
                 runtime_generation=runtime_generation+CASE WHEN runtime_state='stopped' THEN 1 ELSE 0 END,
                 runtime_state='running', presentation_state='headless', work_state='idle',
                 lifecycle_epoch=lifecycle_epoch+CASE WHEN runtime_state='stopped' THEN 1 ELSE 0 END,
                 idle_since=?2, idle_deadline=?3, stopped_at=0, stop_reason=NULL,
                 last_seen=?2, state_changed_at=?2
             WHERE pubkey=?1",
            params![pubkey, now, idle_deadline],
        )?;
        self.conn.execute(
            "UPDATE mcp_actor_aliases SET last_seen=?2 WHERE actor_key=?1",
            params![actor_key, now],
        )?;
        self.conn.execute(
            "UPDATE handle_leases SET live=1, last_active_at=?2 WHERE pubkey=?1",
            params![pubkey, now],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_alias_is_stable_and_never_rebinds() {
        let store = Store::open_memory().unwrap();
        store
            .reserve_handle_for_pubkey("pk-one", "mcp-openai", None, 1)
            .unwrap();
        store
            .reserve_mcp_actor_session("pk-one", "mcp-openai", "mosaico", 1)
            .unwrap();
        store
            .bind_mcp_actor("mcp1_redacted", "openai", "pk-one", 1)
            .unwrap();
        store
            .bind_mcp_actor("mcp1_redacted", "openai", "pk-one", 2)
            .unwrap();
        assert_eq!(
            store.mcp_actor_pubkey("mcp1_redacted").unwrap().as_deref(),
            Some("pk-one")
        );
        assert!(store.is_mcp_actor_pubkey("pk-one").unwrap());
        let session = store.get_session("pk-one").unwrap().unwrap();
        assert_eq!(session.idle_deadline, 1 + HEADLESS_IDLE_TIMEOUT_SECS);
        assert!(store
            .mark_runtime_stopped("pk-one", StopReason::IdleEvicted, 3)
            .unwrap());
        store
            .activate_mcp_actor("mcp1_redacted", "pk-one", 4)
            .unwrap();
        let reactivated = store.get_session("pk-one").unwrap().unwrap();
        assert_eq!(reactivated.runtime_state, RuntimeState::Running);
        assert_eq!(reactivated.runtime_generation, 2);
        assert_eq!(reactivated.idle_deadline, 4 + HEADLESS_IDLE_TIMEOUT_SECS);
        assert!(store
            .mark_runtime_stopped("pk-one", StopReason::IdleEvicted, 5)
            .unwrap());
        assert!(store
            .revoke_session_recovery_if_generation("pk-one", 2)
            .unwrap());
        assert!(store
            .activate_mcp_actor("mcp1_redacted", "pk-one", 6)
            .is_err());
        assert!(store
            .bind_mcp_actor("mcp1_redacted", "openai", "pk-two", 3)
            .is_err());
    }
}
