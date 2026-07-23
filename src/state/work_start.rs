//! Durable handoff from inbox delivery to the exact agent turn that begins work.

use super::*;

const CLAIM_PREFIX: &str = "work-start:";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkStartClaim {
    pub(crate) event_id: String,
    pub(crate) channel_h: String,
}

/// Stage an exact inbox event for the recipient's next work-start boundary.
///
/// This uses `event_claims` rather than a second delivery ledger: the claim is
/// an idempotent, short-lived product-write effect keyed by `(event, recipient)`.
pub(crate) fn stage_from_inbox_tx(
    transaction: &rusqlite::Transaction<'_>,
    rows: &[InboxRow],
    now: u64,
) -> Result<()> {
    for row in rows {
        transaction.execute(
            "INSERT OR IGNORE INTO event_claims
                 (event_id, claim_key, state, from_pubkey, channel_h, body, created_at, updated_at)
            VALUES (?1, ?2, 'pending', ?3, ?4, '', ?5, 0)",
            params![
                &row.event_id,
                claim_key(&row.target_pubkey),
                &row.from_pubkey,
                &row.channel_h,
                now,
            ],
        )?;
    }
    Ok(())
}

impl Store {
    /// Atomically take every unconsumed work-start handoff for one exact session.
    /// A completed claim records the one permissible best-effort reaction attempt.
    pub(crate) fn take_work_start_claims(
        &self,
        target_pubkey: &str,
        now: u64,
    ) -> Result<Vec<WorkStartClaim>> {
        self.take_work_start_claims_for_events(target_pubkey, &[], now)
    }

    /// Atomically take handoffs for the exact inbox rows in one submitted prompt.
    pub(crate) fn take_work_start_claims_for_events(
        &self,
        target_pubkey: &str,
        event_ids: &[String],
        now: u64,
    ) -> Result<Vec<WorkStartClaim>> {
        let key = claim_key(target_pubkey);
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let candidates = if event_ids.is_empty() {
            let mut statement = transaction.prepare(
                "SELECT event_id, channel_h
                 FROM event_claims
                 WHERE claim_key=?1 AND state='pending'
                 ORDER BY created_at, event_id",
            )?;
            let rows = statement.query_map([&key], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let mut statement = transaction.prepare(
                "SELECT channel_h FROM event_claims
                 WHERE event_id=?1 AND claim_key=?2 AND state='pending'",
            )?;
            let mut candidates = Vec::new();
            for event_id in event_ids {
                let channel_h = statement
                    .query_row(params![event_id, &key], |row| row.get(0))
                    .optional()?;
                if let Some(channel_h) = channel_h {
                    candidates.push((event_id.clone(), channel_h));
                }
            }
            candidates
        };

        let mut out = Vec::new();
        for (event_id, channel_h) in candidates {
            let changed = transaction.execute(
                "UPDATE event_claims SET state='completed', updated_at=?3
                 WHERE event_id=?1 AND claim_key=?2
                   AND state='pending'",
                params![&event_id, &key, now],
            )?;
            if changed == 0 {
                continue;
            }
            out.push(WorkStartClaim {
                event_id,
                channel_h,
            });
        }
        transaction.commit()?;
        Ok(out)
    }
}

fn claim_key(target_pubkey: &str) -> String {
    format!("{CLAIM_PREFIX}{target_pubkey}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(store: &Store, rows: &[InboxRow], now: u64) {
        let transaction = rusqlite::Transaction::new_unchecked(
            &store.conn,
            rusqlite::TransactionBehavior::Immediate,
        )
        .unwrap();
        stage_from_inbox_tx(&transaction, rows, now).unwrap();
        transaction.commit().unwrap();
    }

    #[test]
    fn handoff_is_one_shot_for_each_exact_recipient() {
        let store = Store::open_memory().unwrap();
        let a = InboxRow {
            event_id: "event".into(),
            target_pubkey: "agent-a".into(),
            state: "delivered".into(),
            from_pubkey: "human".into(),
            channel_h: "room".into(),
            body: String::new(),
            created_at: 1,
            delivered_at: 2,
        };
        let mut b = a.clone();
        b.target_pubkey = "agent-b".into();
        stage(&store, &[a.clone(), b], 3);
        stage(&store, &[a], 4);

        assert_eq!(
            store.take_work_start_claims("agent-a", 5).unwrap(),
            [WorkStartClaim {
                event_id: "event".into(),
                channel_h: "room".into(),
            }]
        );
        assert!(store
            .take_work_start_claims("agent-a", 6)
            .unwrap()
            .is_empty());
        assert_eq!(store.take_work_start_claims("agent-b", 7).unwrap().len(), 1);
    }

    #[test]
    fn submitted_prompt_cannot_consume_another_prompts_handoff() {
        let store = Store::open_memory().unwrap();
        let rows = ["first", "second"].map(|event_id| InboxRow {
            event_id: event_id.into(),
            target_pubkey: "agent".into(),
            state: "injected".into(),
            from_pubkey: "human".into(),
            channel_h: "room".into(),
            body: String::new(),
            created_at: 1,
            delivered_at: 2,
        });
        stage(&store, &rows, 3);

        let first = store
            .take_work_start_claims_for_events("agent", &["first".into()], 4)
            .unwrap();
        assert_eq!(first[0].event_id, "first");
        assert_eq!(
            store.take_work_start_claims("agent", 5).unwrap()[0].event_id,
            "second"
        );
    }
}
