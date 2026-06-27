//! Durable ordinal identities + `(pubkey, h)` route bindings (issue #47).
//!
//! An agent has a deterministic identity series: ordinal 0 is the base
//! file-backed key; ordinal N>0 is an HKDF tweak of the base secret, reused
//! across rooms. The authoritative resume key is `(pubkey, h)` where `h` is the
//! session's `route_scope` (channel when set, else per-session room). The native
//! harness session id is a STORED ATTRIBUTE of that occupancy, so a mention to an
//! offline ordinal can resume its bound native session.

use super::*;

impl Store {
    // ── ordinal identities + routes (issue #47) ─────────────────────────────

    /// Record an ordinal identity in the durable local inventory (idempotent).
    /// Ordinal 0 (the base agent) may also be recorded so `#p` enumeration is
    /// uniform, but callers typically only persist N>0 since base pubkeys come
    /// from the keystore.
    pub fn ensure_agent_ordinal(
        &self,
        base_pubkey: &str,
        agent_slug: &str,
        ordinal: u32,
        pubkey: &str,
        created_at: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO agent_ordinals (base_pubkey, agent_slug, ordinal, pubkey, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(base_pubkey, ordinal) DO UPDATE SET pubkey=?4, agent_slug=?2",
            params![base_pubkey, agent_slug, ordinal, pubkey, created_at],
        )?;
        Ok(())
    }

    /// Every ordinal pubkey ever allocated locally — the durable contribution to
    /// the subscription `#p` set. Base (ordinal-0) pubkeys are added separately
    /// from the keystore by the caller.
    pub fn list_agent_ordinal_pubkeys(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(mut stmt) = self.conn.prepare("SELECT pubkey FROM agent_ordinals") {
            if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
                out.extend(rows.flatten());
            }
        }
        out
    }

    /// Reverse-map an ordinal pubkey to its `(base_pubkey, agent_slug, ordinal)`.
    /// `None` when the pubkey is not a known local ordinal identity.
    pub fn local_agent_ordinal_for_pubkey(&self, pubkey: &str) -> Option<(String, String, u32)> {
        self.conn
            .query_row(
                "SELECT base_pubkey, agent_slug, ordinal FROM agent_ordinals WHERE pubkey=?1",
                params![pubkey],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, u32>(2)?,
                    ))
                },
            )
            .ok()
    }

    /// Insert or replace the route binding for `(pubkey, h)`. On channel-switch
    /// the binding MOVES (its `h` changes) — handled by `move_identity_route`,
    /// not here. This is the create/rebind primitive used at session start and
    /// on resume.
    pub fn upsert_identity_route(&self, r: &IdentityRoute, bound_at: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO session_identity_routes
               (pubkey, h, session_id, base_pubkey, agent_slug, ordinal, label,
                harness_kind, native_id, alive, bound_at, released_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,0)
             ON CONFLICT(pubkey, h) DO UPDATE SET
               session_id=?3, base_pubkey=?4, agent_slug=?5, ordinal=?6, label=?7,
               harness_kind=?8, native_id=?9, alive=?10, bound_at=?11, released_at=0",
            params![
                r.pubkey,
                r.h,
                r.session_id,
                r.base_pubkey,
                r.agent_slug,
                r.ordinal,
                r.label,
                r.harness_kind,
                r.native_id,
                r.alive as i64,
                bound_at,
            ],
        )?;
        Ok(())
    }

    fn read_identity_route(row: &rusqlite::Row) -> rusqlite::Result<IdentityRoute> {
        Ok(IdentityRoute {
            pubkey: row.get("pubkey")?,
            h: row.get("h")?,
            session_id: row.get("session_id")?,
            base_pubkey: row.get("base_pubkey")?,
            agent_slug: row.get("agent_slug")?,
            ordinal: row.get("ordinal")?,
            label: row.get("label")?,
            harness_kind: row.get("harness_kind")?,
            native_id: row.get("native_id")?,
            alive: row.get::<_, i64>("alive")? != 0,
        })
    }

    /// The LIVE route for `(pubkey, h)`, if a session currently occupies it.
    pub fn live_identity_route(&self, pubkey: &str, h: &str) -> Option<IdentityRoute> {
        self.conn
            .query_row(
                "SELECT * FROM session_identity_routes WHERE pubkey=?1 AND h=?2 AND alive=1",
                params![pubkey, h],
                Self::read_identity_route,
            )
            .ok()
    }

    /// Any route for `(pubkey, h)` — alive or dead. A dead row carries the bound
    /// `native_id` so a mention can resume it.
    pub fn bound_identity_route(&self, pubkey: &str, h: &str) -> Option<IdentityRoute> {
        self.conn
            .query_row(
                "SELECT * FROM session_identity_routes WHERE pubkey=?1 AND h=?2",
                params![pubkey, h],
                Self::read_identity_route,
            )
            .ok()
    }

    /// The route bound to a specific session id (one per session).
    pub fn identity_route_for_session(&self, session_id: &str) -> Option<IdentityRoute> {
        self.conn
            .query_row(
                "SELECT * FROM session_identity_routes WHERE session_id=?1",
                params![session_id],
                Self::read_identity_route,
            )
            .ok()
    }

    /// Ordinals currently LIVE for `base_pubkey` in room `h` (used to pick the
    /// lowest free ordinal at birth). `except_session` excludes a session being
    /// reassigned so it doesn't block itself.
    pub fn live_ordinals_in_h(
        &self,
        base_pubkey: &str,
        h: &str,
        except_session: Option<&str>,
    ) -> Vec<u32> {
        let mut out = Vec::new();
        if let Ok(mut stmt) = self.conn.prepare(
            "SELECT ordinal, session_id FROM session_identity_routes
             WHERE base_pubkey=?1 AND h=?2 AND alive=1",
        ) {
            if let Ok(rows) = stmt.query_map(params![base_pubkey, h], |r| {
                Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?))
            }) {
                for (ord, sid) in rows.flatten() {
                    if except_session == Some(sid.as_str()) {
                        continue;
                    }
                    out.push(ord);
                }
            }
        }
        out
    }

    /// Mark a session's route dead (session ended/crashed), keeping the row so a
    /// future mention to `(pubkey, h)` can resume the bound native session.
    pub fn mark_identity_route_dead(&self, session_id: &str, ts: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE session_identity_routes SET alive=0, released_at=?2 WHERE session_id=?1",
            params![session_id, ts],
        )?;
        Ok(())
    }

    /// Move a live session's route to a new room `dst_h` (channel switch). The
    /// ordinal pubkey is fixed; only `h` changes. `(pubkey, dst_h)` becomes the
    /// new resume key. Returns false if the session has no route.
    pub fn move_identity_route(&self, session_id: &str, dst_h: &str, ts: u64) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE session_identity_routes SET h=?2, bound_at=?3 WHERE session_id=?1",
            params![session_id, dst_h, ts],
        )?;
        Ok(n > 0)
    }
}
