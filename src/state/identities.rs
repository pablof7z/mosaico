//! `identities` — per-session minted keys the daemon publishes as.
//!
//! Each row maps a session's own pubkey to its owning session, its codename, and
//! a resume binding. Bounds the `#p` subscription (the set of pubkeys the daemon
//! listens for) and resumes the right session when a mention arrives for an
//! offline agent.

use super::*;

const COLS: &str = "pubkey, agent_slug, codename, session_id, channel_h, native_id, \
     alive, created_at";

fn row_to_identity(row: &rusqlite::Row) -> rusqlite::Result<Identity> {
    Ok(Identity {
        pubkey: row.get(0)?,
        agent_slug: row.get(1)?,
        codename: row.get(2)?,
        session_id: row.get(3)?,
        channel_h: row.get(4)?,
        native_id: row.get(5)?,
        alive: row.get::<_, i64>(6)? != 0,
        created_at: row.get(7)?,
    })
}

impl Store {
    /// Upsert a per-session identity keyed by `(pubkey, session_id)`.
    pub fn upsert_identity(&self, i: &Identity) -> Result<()> {
        if !i.session_id.is_empty() {
            self.conn.execute(
                "DELETE FROM identities WHERE session_id=?1 AND pubkey<>?2",
                params![i.session_id, i.pubkey],
            )?;
        }
        self.conn.execute(
            "INSERT INTO identities
                 (pubkey, agent_slug, codename, session_id, channel_h, native_id,
                  alive, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(pubkey, session_id) DO UPDATE SET
                 agent_slug=excluded.agent_slug, codename=excluded.codename,
                 session_id=excluded.session_id,
                 channel_h=excluded.channel_h, native_id=excluded.native_id,
                 alive=excluded.alive",
            params![
                i.pubkey,
                i.agent_slug,
                i.codename,
                i.session_id,
                i.channel_h,
                i.native_id,
                i.alive as i64,
                i.created_at
            ],
        )?;
        Ok(())
    }

    /// Fetch the newest known identity row for a derived pubkey.
    pub fn get_identity(&self, pubkey: &str) -> Result<Option<Identity>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {COLS} FROM identities WHERE pubkey=?1
                     ORDER BY alive DESC, created_at DESC LIMIT 1"
                ),
                params![pubkey],
                row_to_identity,
            )
            .optional()?)
    }

    /// Fetch the identity row for a derived pubkey in a specific channel.
    pub fn get_identity_for_channel(
        &self,
        pubkey: &str,
        channel_h: &str,
    ) -> Result<Option<Identity>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {COLS} FROM identities
                     WHERE pubkey=?1 AND channel_h=?2
                     ORDER BY alive DESC, created_at DESC LIMIT 1"
                ),
                params![pubkey, channel_h],
                row_to_identity,
            )
            .optional()?)
    }

    /// Every minted pubkey the daemon signs as — the bound on the `#p`
    /// subscription.
    pub fn list_identity_pubkeys(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT pubkey FROM identities ORDER BY created_at")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Bind a derived identity to a live harness session for resume: records the
    /// canonical session id (resolved first), the harness-native id, and liveness.
    pub fn bind_session_identity(
        &self,
        pubkey: &str,
        session_id: &str,
        native_id: &str,
        alive: bool,
    ) -> Result<()> {
        let canonical = self
            .resolve_canonical_id(session_id)?
            .unwrap_or_else(|| session_id.to_string());
        self.conn.execute(
            "DELETE FROM identities WHERE session_id=?1 AND pubkey<>?2",
            params![canonical, pubkey],
        )?;
        let changed = self.conn.execute(
            "UPDATE identities SET native_id=?3, alive=?4
             WHERE pubkey=?1 AND session_id=?2",
            params![pubkey, canonical, native_id, alive as i64],
        )?;
        if changed == 0 {
            if let Some(mut identity) = self.get_identity(pubkey)? {
                identity.session_id = canonical;
                identity.native_id = native_id.to_string();
                identity.alive = alive;
                self.upsert_identity(&identity)?;
            }
        }
        Ok(())
    }

    /// The identity currently bound to a canonical session (resolves the id
    /// first). Used by `who`/signing/reconcile to report a session's selected
    /// ordinal identity. Newest binding wins.
    pub fn identity_for_session(&self, session_id: &str) -> Result<Option<Identity>> {
        let Some(canonical) = self.resolve_canonical_id(session_id)? else {
            return Ok(None);
        };
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {COLS} FROM identities WHERE session_id=?1
                     ORDER BY created_at DESC LIMIT 1"
                ),
                params![canonical],
                row_to_identity,
            )
            .optional()?)
    }

    /// The [`crate::identity::SessionIdentity`] for a session: the projection of
    /// its bound `identities` row that read-side callers (status/chat publish,
    /// `who`/statusline, mention routing) consume instead of re-deriving
    /// label/pubkey at the edge. `None` when no identity row is bound yet
    /// (callers fall back to the session row).
    pub fn session_identity_for_session(
        &self,
        session_id: &str,
    ) -> Result<Option<crate::identity::SessionIdentity>> {
        Ok(self
            .identity_for_session(session_id)?
            .map(|i| crate::identity::SessionIdentity::new(i.pubkey, i.agent_slug, i.codename)))
    }

    /// Mark every identity bound to a session dead (alive=0) while KEEPING the row
    /// so a later mention can resume its bound native session. Resolves the id.
    pub fn mark_identity_dead_for_session(&self, session_id: &str) -> Result<()> {
        let Some(canonical) = self.resolve_canonical_id(session_id)? else {
            return Ok(());
        };
        self.conn.execute(
            "UPDATE identities SET alive=0 WHERE session_id=?1",
            params![canonical],
        )?;
        Ok(())
    }

    /// The identity bound to an exact selected pubkey in a given channel.
    pub fn resolve_identity_pubkey_for_channel(
        &self,
        pubkey: &str,
        channel_h: &str,
    ) -> Result<Option<Identity>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {COLS} FROM identities
                     WHERE pubkey=?1 AND channel_h=?2
                     ORDER BY alive DESC, created_at DESC LIMIT 1"
                ),
                params![pubkey, channel_h],
                row_to_identity,
            )
            .optional()?)
    }
}

#[cfg(test)]
#[path = "identities/tests.rs"]
mod tests;
