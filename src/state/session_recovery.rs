//! Two-phase destruction of exact-session recovery authority.

use super::*;
use rusqlite::{Transaction, TransactionBehavior};

impl Store {
    /// Durably prohibit another runtime reservation for this exact generation.
    /// Runtime locators, signers, and routes remain intact so a failed process
    /// termination can be inspected and retried safely.
    pub fn revoke_session_recovery_if_generation(
        &self,
        pubkey: &str,
        runtime_generation: u64,
    ) -> Result<bool> {
        Ok(self.conn.execute(
            "UPDATE sessions SET recovery_state='revoked'
             WHERE pubkey=?1 AND runtime_generation=?2",
            params![pubkey, runtime_generation],
        )? == 1)
    }

    /// Finalize an already-revoked generation after its process termination is
    /// confirmed. This is deliberately idempotent with respect to the revoked
    /// state so a failed or interrupted first attempt can finish on retry.
    pub fn finalize_session_recovery_revocation(
        &self,
        pubkey: &str,
        runtime_generation: u64,
        now: u64,
    ) -> Result<bool> {
        let transaction = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE sessions
             SET runtime_state='stopped', presentation_state='unavailable', work_state='idle',
                 lifecycle_epoch=lifecycle_epoch+1,
                 idle_since=0, idle_deadline=0, stopped_at=?3,
                 stop_reason='revoked', turn_started_at=0, state_changed_at=?3
             WHERE pubkey=?1 AND runtime_generation=?2 AND recovery_state='revoked'",
            params![pubkey, runtime_generation, now],
        )?;
        if changed == 0 {
            transaction.rollback()?;
            return Ok(false);
        }
        transaction.execute(
            "UPDATE handle_leases SET live=0, last_active_at=MAX(last_active_at, ?2)
             WHERE pubkey=?1",
            params![pubkey, now],
        )?;
        let lifecycle_epoch: u64 = transaction.query_row(
            "SELECT lifecycle_epoch FROM sessions WHERE pubkey=?1",
            [pubkey],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO session_standing
                 (pubkey, channel_h, state, retain_until, standing_epoch,
                  session_lifecycle_epoch, updated_at)
             SELECT route.pubkey, route.channel_h, 'absent', 0, 1, ?2, ?3
               FROM session_channels route WHERE route.pubkey=?1
             ON CONFLICT(pubkey, channel_h) DO NOTHING",
            params![pubkey, lifecycle_epoch, now],
        )?;
        transaction.execute(
            "UPDATE session_standing
             SET state=CASE WHEN state='absent' THEN 'absent' ELSE 'retained' END,
                 retain_until=CASE WHEN state='absent' THEN 0 ELSE ?3 END,
                 standing_epoch=standing_epoch+1,
                 session_lifecycle_epoch=?2, updated_at=?3
             WHERE pubkey=?1",
            params![pubkey, lifecycle_epoch, now],
        )?;
        transaction.execute("DELETE FROM session_locators WHERE pubkey=?1", [pubkey])?;
        transaction.execute("DELETE FROM session_signers WHERE pubkey=?1", [pubkey])?;
        transaction.execute("DELETE FROM session_channels WHERE pubkey=?1", [pubkey])?;
        transaction.commit()?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registration(at: u64) -> RegisterSession {
        RegisterSession {
            pubkey: "pk".into(),
            observed_harness: "codex".into(),
            agent_slug: "codex".into(),
            channel_h: "root".into(),
            child_pid: None,
            transcript_path: None,
            now: at,
        }
    }

    fn reserve(store: &Store, at: u64) -> Result<u64> {
        store.reserve_session_with_facts(
            &registration(at),
            &AdmittedRuntimeFacts {
                observed_harness: "codex".into(),
                claimed_harness: String::new(),
                bundle: "codex-pty".into(),
                transport: "pty".into(),
                endpoint_provenance: "launch".into(),
            },
        )
    }

    #[test]
    fn revocation_fence_survives_exit_before_finalize_and_blocks_respawn() {
        let store = Store::open_memory().unwrap();
        let generation = reserve(&store, 1).unwrap();
        store.bind_session_signer("pk", "salt").unwrap();
        store
            .put_session_locator("codex", LOCATOR_PTY, "pty-1", "pk", 2)
            .unwrap();

        assert!(store
            .revoke_session_recovery_if_generation("pk", generation)
            .unwrap());
        assert!(!store
            .bind_runtime_process("pk", generation, Some(42))
            .unwrap());
        assert!(store
            .mark_runtime_stopped_if_generation("pk", generation, StopReason::Crash, 3)
            .unwrap());
        assert!(reserve(&store, 4).is_err());
        assert!(store
            .runtime_locator_for_session("pk", generation, LOCATOR_PTY)
            .unwrap()
            .is_some());
        assert!(store.session_signer_salt("pk").unwrap().is_some());
        assert!(!store.list_session_routes("pk").unwrap().is_empty());

        assert!(store
            .finalize_session_recovery_revocation("pk", generation, 5)
            .unwrap());
        assert!(store.locators_for_pubkey("pk").unwrap().is_empty());
        assert!(store.list_session_routes("pk").unwrap().is_empty());
        assert!(store.session_signer_salt("pk").unwrap().is_none());
    }

    #[test]
    fn stale_generation_cannot_revoke_a_new_current_runtime() {
        let store = Store::open_memory().unwrap();
        let first = reserve(&store, 1).unwrap();
        store
            .mark_runtime_stopped_if_generation("pk", first, StopReason::Crash, 2)
            .unwrap();
        let second = reserve(&store, 3).unwrap();

        assert!(!store
            .revoke_session_recovery_if_generation("pk", first)
            .unwrap());
        assert!(store
            .revoke_session_recovery_if_generation("pk", second)
            .unwrap());
        assert_eq!(
            store.get_session("pk").unwrap().unwrap().recovery_state,
            RecoveryState::Revoked
        );
    }

    #[test]
    fn finalize_requires_a_durable_revocation_fence() {
        let store = Store::open_memory().unwrap();
        let generation = reserve(&store, 1).unwrap();

        assert!(!store
            .finalize_session_recovery_revocation("pk", generation, 2)
            .unwrap());
        assert!(store.get_session("pk").unwrap().unwrap().is_running());
    }
}
