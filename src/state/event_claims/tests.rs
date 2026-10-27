use super::*;

fn state_for(store: &Store, event_id: &str, claim_key: &str) -> String {
    store
        .conn
        .query_row(
            "SELECT state FROM event_claims WHERE event_id=?1 AND claim_key=?2",
            params![event_id, claim_key],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn orchestration_claim_survives_store_reopen_until_lease_expires() {
    // The #837 "no per-boot re-side-effects" requirement for the durable
    // event+recipient claim: an orchestration target claimed (processing)
    // in a prior daemon run is NOT re-claimed on restart while its lease is
    // live — the durable `event_claims` row is the cross-restart guard that
    // retires `first_sight`'s per-process LRU. Only once the processing
    // lease expires may a restart re-claim it (the recovery path).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let claim_key = "orchestration:backend:0:a";

    {
        let s = Store::open(&path).unwrap();
        assert!(s
            .claim_orchestration_target("ev", claim_key, "admin", "child", "a", 100)
            .unwrap());
    }

    // Restart: re-claiming the same target within the processing lease is
    // rejected — the durable claim survives the reopen.
    let reopened = Store::open(&path).unwrap();
    assert!(
        !reopened
            .claim_orchestration_target("ev", claim_key, "admin", "child", "a", 101)
            .unwrap(),
        "restart must not re-claim a live processing target"
    );
    assert_eq!(state_for(&reopened, "ev", claim_key), "processing");

    // Past the processing lease, a restart may re-claim a stuck target
    // (crash recovery), but a completed target is never re-claimed.
    let stale_now = 100 + PROCESSING_LEASE_SECS + 1;
    assert!(
        reopened
            .claim_orchestration_target("ev", claim_key, "admin", "child", "a", stale_now)
            .unwrap(),
        "a target stuck processing past its lease is re-claimable on restart"
    );
    reopened
        .complete_orchestration_target("ev", claim_key, stale_now)
        .unwrap();
    assert!(
        !reopened
            .claim_orchestration_target("ev", claim_key, "admin", "child", "a", stale_now + 1)
            .unwrap(),
        "a completed target is never re-claimed"
    );
}

#[test]
fn orchestration_claim_retries_only_failed_targets() {
    let store = Store::open_memory().unwrap();
    let a = "orchestration:backend:0:a";
    let b = "orchestration:backend:1:b";
    assert!(store
        .claim_orchestration_target("ev", a, "admin", "child", "a", 10)
        .unwrap());
    assert!(store
        .claim_orchestration_target("ev", b, "admin", "child", "b", 10)
        .unwrap());

    store.retry_orchestration_target("ev", a, 11).unwrap();
    store.complete_orchestration_target("ev", b, 11).unwrap();

    assert!(store
        .claim_orchestration_target("ev", a, "admin", "child", "a", 11 + RETRY_BASE_DELAY_SECS)
        .unwrap());
    assert!(!store
        .claim_orchestration_target("ev", b, "admin", "child", "b", 11 + RETRY_BASE_DELAY_SECS)
        .unwrap());
    assert_eq!(state_for(&store, "ev", a), "processing");
    assert_eq!(state_for(&store, "ev", b), "completed");
}

#[test]
fn failed_orchestration_target_waits_out_its_retry_delay() {
    let store = Store::open_memory().unwrap();
    let target = "orchestration:backend:0:a";
    assert!(store
        .claim_orchestration_target("ev", target, "admin", "child", "a", 100)
        .unwrap());
    store.retry_orchestration_target("ev", target, 100).unwrap();

    assert!(
        !store
            .claim_orchestration_target("ev", target, "admin", "child", "a", 100)
            .unwrap(),
        "a relay replay right after the failure must not re-claim the target"
    );
    assert!(!store
        .claim_orchestration_target(
            "ev",
            target,
            "admin",
            "child",
            "a",
            100 + RETRY_BASE_DELAY_SECS - 1,
        )
        .unwrap());
    assert_eq!(state_for(&store, "ev", target), "pending");
    assert!(store
        .claim_orchestration_target(
            "ev",
            target,
            "admin",
            "child",
            "a",
            100 + RETRY_BASE_DELAY_SECS
        )
        .unwrap());
    assert_eq!(state_for(&store, "ev", target), "processing");
}

#[test]
fn offline_mention_replay_cannot_bypass_its_retry_delay() {
    let store = Store::open_memory().unwrap();
    let target = "agent";

    assert!(store
        .claim_offline_mention("event", target, "sender", "room", "body", 100)
        .unwrap());
    store.retry_offline_mention("event", target, 100).unwrap();

    assert!(
        !store
            .claim_offline_mention("event", target, "sender", "room", "body", 101)
            .unwrap(),
        "a retained relay event must not retry a failed mention before the durable backoff"
    );
    assert!(!store
        .claim_offline_mention(
            "event",
            target,
            "sender",
            "room",
            "body",
            100 + RETRY_BASE_DELAY_SECS - 1,
        )
        .unwrap());
    assert!(store
        .claim_offline_mention(
            "event",
            target,
            "sender",
            "room",
            "body",
            100 + RETRY_BASE_DELAY_SECS,
        )
        .unwrap());
}

/// Test-only access for daemon-level retry tests.
impl Store {
    /// `(state, attempts, from_pubkey)` of the event's claim whose key starts
    /// with `prefix`.
    pub(crate) fn event_claim_for_test(
        &self,
        event_id: &str,
        claim_prefix: &str,
    ) -> Option<(String, u32, String)> {
        self.conn
            .query_row(
                "SELECT state, attempts, from_pubkey FROM event_claims
                 WHERE event_id=?1 AND claim_key LIKE ?2",
                params![event_id, format!("{claim_prefix}%")],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok()
    }

    /// Blank the stored requester. Only a claim writes it back, so seeing it
    /// again proves a handler claimed (and so ran) the target.
    pub(crate) fn scrub_event_claim_requester_for_test(&self, event_id: &str) {
        self.conn
            .execute(
                "UPDATE event_claims SET from_pubkey='' WHERE event_id=?1",
                [event_id],
            )
            .unwrap();
    }

    /// Shift the event's claim timestamps `secs` into the past, standing in
    /// for wall-clock time passing.
    pub(crate) fn rewind_event_claims_for_test(&self, event_id: &str, secs: u64) {
        self.conn
            .execute(
                "UPDATE event_claims
                 SET created_at=created_at-?2, updated_at=updated_at-?2,
                     retry_at=MAX(retry_at-?2, 0)
                 WHERE event_id=?1",
                params![event_id, secs],
            )
            .unwrap();
    }

    /// Make every write fail while reads still succeed (as on a full disk).
    pub(crate) fn set_read_only_for_test(&self, read_only: bool) {
        self.conn
            .pragma_update(None, "query_only", read_only)
            .unwrap();
    }

    /// Make event-claim reads fail by hiding the table, or restore it.
    pub(crate) fn hide_event_claims_for_test(&self, hidden: bool) {
        let (from, to) = if hidden {
            ("event_claims", "event_claims_hidden")
        } else {
            ("event_claims_hidden", "event_claims")
        };
        self.conn
            .execute_batch(&format!("ALTER TABLE {from} RENAME TO {to}"))
            .unwrap();
    }

    /// Rewind one claim's lease and schedule, standing in for time passing.
    pub(crate) fn rewind_claim_for_test(&self, event_id: &str, claim_key: &str, secs: u64) {
        self.conn
            .execute(
                "UPDATE event_claims
                 SET updated_at=updated_at-?3, retry_at=MAX(retry_at-?3, 0)
                 WHERE event_id=?1 AND claim_key=?2",
                params![event_id, claim_key, secs],
            )
            .unwrap();
    }

    /// Set a claim's stored schedule directly.
    pub(crate) fn set_claim_schedule_for_test(
        &self,
        event_id: &str,
        updated_at: u64,
        retry_at: u64,
    ) {
        self.conn
            .execute(
                "UPDATE event_claims SET updated_at=?2, retry_at=?3 WHERE event_id=?1",
                params![event_id, updated_at, retry_at],
            )
            .unwrap();
    }

    /// Make every write to `event_id`'s claims fail, as a per-row store error.
    pub(crate) fn fail_writes_for_event_for_test(&self, event_id: &str) {
        self.conn
            .execute_batch(&format!(
                "CREATE TRIGGER fail_{event_id} BEFORE UPDATE ON event_claims
                 WHEN OLD.event_id='{event_id}'
                 BEGIN SELECT RAISE(ABORT, 'injected store error'); END;"
            ))
            .unwrap();
    }
}
