//! `relay_reactions` — the NIP-25 reaction projection.
//!
//! A reaction row is materialized ONLY from a round-tripped kind:7 relay event
//! (see `Nip29Materializer::materialize_reaction`); it is never optimistically
//! fabricated. Reactions are passive awareness: they are surfaced at turn-start
//! and never routed to the inbox, so no delivery/doorbell path ever reads them.

use super::*;

impl Store {
    /// Upsert one reaction. Idempotent by `reaction_id` (the kind:7 event id), so a
    /// relay echo of a locally seeded reaction collapses onto the same row.
    pub fn upsert_reaction(
        &self,
        reaction_id: &str,
        target_message_id: &str,
        channel_h: &str,
        reactor_pubkey: &str,
        emoji: &str,
        created_at: u64,
    ) -> Result<bool> {
        let changed = self.conn.execute(
            "INSERT INTO relay_reactions
                 (reaction_id, target_message_id, channel_h, reactor_pubkey, emoji, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(reaction_id) DO NOTHING",
            params![
                reaction_id,
                target_message_id,
                channel_h,
                reactor_pubkey,
                emoji,
                created_at
            ],
        )?;
        Ok(changed > 0)
    }

    /// Reactions on messages authored by `author_pubkey`, created strictly after
    /// `since`, excluding the author's own reactions. Joined to `messages` for the
    /// target body so awareness can render a snippet. Oldest-first, capped.
    pub fn reactions_on_authored_after(
        &self,
        author_pubkey: &str,
        since: u64,
        limit: u32,
    ) -> Result<Vec<ReactionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.reaction_id, r.target_message_id, r.channel_h, r.reactor_pubkey,
                    r.emoji, r.created_at, m.body
             FROM relay_reactions r
             JOIN messages m ON m.message_id = r.target_message_id
             WHERE m.author_pubkey = ?1
               AND r.created_at > ?2
               AND r.reactor_pubkey <> ?1
             ORDER BY r.created_at ASC, r.reaction_id ASC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![author_pubkey, since, limit], |row| {
            Ok(ReactionRow {
                reaction_id: row.get(0)?,
                target_message_id: row.get(1)?,
                channel_h: row.get(2)?,
                reactor_pubkey: row.get(3)?,
                emoji: row.get(4)?,
                created_at: row.get(5)?,
                target_body: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Newest reactions in one channel, used as a bounded engagement signal.
    pub fn recent_reactions_for_channel(
        &self,
        channel_h: &str,
        since: u64,
        limit: u32,
    ) -> Result<Vec<ReactionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.reaction_id, r.target_message_id, r.channel_h, r.reactor_pubkey,
                    r.emoji, r.created_at, m.body
             FROM relay_reactions r
             JOIN messages m ON m.message_id = r.target_message_id
             WHERE r.channel_h = ?1 AND r.created_at > ?2
             ORDER BY r.created_at DESC, r.reaction_id DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![channel_h, since, limit], |row| {
            Ok(ReactionRow {
                reaction_id: row.get(0)?,
                target_message_id: row.get(1)?,
                channel_h: row.get(2)?,
                reactor_pubkey: row.get(3)?,
                emoji: row.get(4)?,
                created_at: row.get(5)?,
                target_body: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn has_reaction_from_pubkey_on_message(
        &self,
        target_message_id: &str,
        reactor_pubkey: &str,
    ) -> Result<bool> {
        let exists: i64 = self.conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM relay_reactions
                 WHERE target_message_id=?1 AND reactor_pubkey=?2
             )",
            params![target_message_id, reactor_pubkey],
            |row| row.get(0),
        )?;
        Ok(exists != 0)
    }
}

#[cfg(test)]
mod tests {
    use crate::state::{RecordMessage, Store};

    fn record_msg(store: &Store, id: &str, author: &str, body: &str, created_at: u64) {
        store
            .record_message(&RecordMessage {
                message_id: id.into(),
                thread_id: "chan".into(),
                channel_h: "chan".into(),
                author_pubkey: author.into(),
                body: body.into(),
                created_at,
                direction: "outbound".into(),
                sync_state: "accepted".into(),
                native_event_id: Some(id.into()),
                error: None,
            })
            .unwrap();
    }

    #[test]
    fn upsert_is_idempotent_by_reaction_id() {
        let store = Store::open_memory().unwrap();
        assert!(store
            .upsert_reaction("rx1", "msg1", "chan", "peer", "👍", 10)
            .unwrap());
        // Second insert of the same reaction id is a no-op.
        assert!(!store
            .upsert_reaction("rx1", "msg1", "chan", "peer", "👍", 10)
            .unwrap());
    }

    #[test]
    fn query_filters_cursor_self_and_foreign_targets() {
        let store = Store::open_memory().unwrap();
        record_msg(&store, "mine", "me", "pushed the fix", 5);
        record_msg(&store, "theirs", "other", "unrelated", 5);

        // A peer reaction on my message after the cursor: visible.
        store
            .upsert_reaction("rx-visible", "mine", "chan", "peer", "👍", 20)
            .unwrap();
        // A reaction before the cursor: filtered by `since`.
        store
            .upsert_reaction("rx-old", "mine", "chan", "peer", "🎉", 8)
            .unwrap();
        // My own reaction on my message: excluded.
        store
            .upsert_reaction("rx-self", "mine", "chan", "me", "✅", 20)
            .unwrap();
        // A reaction on someone else's message: excluded by the author join.
        store
            .upsert_reaction("rx-foreign", "theirs", "chan", "peer", "👀", 20)
            .unwrap();

        let rows = store.reactions_on_authored_after("me", 10, 50).unwrap();
        assert_eq!(rows.len(), 1, "only the visible peer reaction survives");
        assert_eq!(rows[0].reaction_id, "rx-visible");
        assert_eq!(rows[0].emoji, "👍");
        assert_eq!(rows[0].target_body, "pushed the fix");
    }

    #[test]
    fn recent_channel_reactions_limit_keeps_the_newest_rows() {
        let store = Store::open_memory().unwrap();
        record_msg(&store, "target", "author", "decision", 1);
        for (id, at) in [("old", 10), ("middle", 20), ("new", 30)] {
            store
                .upsert_reaction(id, "target", "chan", "reactor", "👍", at)
                .unwrap();
        }

        let rows = store.recent_reactions_for_channel("chan", 0, 2).unwrap();
        assert_eq!(
            rows.iter()
                .map(|reaction| reaction.reaction_id.as_str())
                .collect::<Vec<_>>(),
            ["new", "middle"]
        );
    }
}
