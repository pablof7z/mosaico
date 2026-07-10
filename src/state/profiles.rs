//! `relay_profiles` — kind:0 metadata cache, keyed by pubkey.

use super::*;

fn row_to_profile(row: &rusqlite::Row) -> rusqlite::Result<Profile> {
    Ok(Profile {
        pubkey: row.get(0)?,
        name: row.get(1)?,
        slug: row.get(2)?,
        agent_slug: row.get(3)?,
        host: row.get(4)?,
        is_backend: row.get::<_, i64>(5)? != 0,
        updated_at: row.get(6)?,
    })
}

const COLS: &str = "pubkey, name, slug, agent_slug, host, is_backend, updated_at";

impl Store {
    /// Materialize a kind:0 profile. Newer `updated_at` wins.
    pub fn upsert_profile(
        &self,
        pubkey: &str,
        name: &str,
        slug: &str,
        host: &str,
        is_backend: bool,
        updated_at: u64,
    ) -> Result<()> {
        self.upsert_profile_with_agent_slug(pubkey, name, slug, "", host, is_backend, updated_at)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_profile_with_agent_slug(
        &self,
        pubkey: &str,
        name: &str,
        slug: &str,
        agent_slug: &str,
        host: &str,
        is_backend: bool,
        updated_at: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO relay_profiles
                 (pubkey, name, slug, agent_slug, host, is_backend, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(pubkey) DO UPDATE SET
                 name=excluded.name, slug=excluded.slug,
                 agent_slug=excluded.agent_slug, host=excluded.host,
                 is_backend=excluded.is_backend, updated_at=excluded.updated_at
             WHERE excluded.updated_at >= relay_profiles.updated_at",
            params![
                pubkey,
                name,
                slug,
                agent_slug,
                host,
                is_backend as i64,
                updated_at
            ],
        )?;
        Ok(())
    }

    /// Fetch one pubkey's profile.
    pub fn get_profile(&self, pubkey: &str) -> Result<Option<Profile>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLS} FROM relay_profiles WHERE pubkey=?1"),
                params![pubkey],
                row_to_profile,
            )
            .optional()?)
    }

    /// Reverse lookup: the pubkey of an agent advertising `slug` on the exact
    /// config.json `backendName` label. Non-backend profiles only.
    pub fn resolve_agent_pubkey(&self, slug: &str, host: &str) -> Result<Option<String>> {
        let name = crate::idref::agent_label(slug, host);
        let mut stmt = self.conn.prepare(
            "SELECT pubkey, host FROM relay_profiles
                 WHERE is_backend=0 AND (slug=?1 OR name=?2)",
        )?;
        let rows = stmt.query_map(params![slug, name], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (pk, h) = row?;
            if h == host {
                return Ok(Some(pk));
            }
        }
        Ok(None)
    }

    /// Reverse lookup for the public per-session handle (`agent/session`).
    pub fn resolve_profile_handle_pubkey(&self, handle: &str) -> Result<Option<String>> {
        let handle = handle.trim();
        if crate::idref::parse_session_handle(handle).is_none() {
            return Ok(None);
        }
        Ok(self
            .conn
            .query_row(
                "SELECT pubkey FROM relay_profiles
                 WHERE is_backend=0 AND (name=?1 OR slug=?1)
                 ORDER BY updated_at DESC LIMIT 1",
                params![handle],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }

    /// Reverse lookup: the pubkey of a backend with exactly this config
    /// `backendName` label. Invite/orchestration surfaces do not accept OS/DNS
    /// hostnames, pubkeys, NIP-05, or slugified display strings here.
    pub fn pubkey_for_backend_label(&self, backend_label: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT pubkey, host FROM relay_profiles WHERE is_backend=1")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (pk, h) = row?;
            if h == backend_label {
                return Ok(Some(pk));
            }
        }
        Ok(None)
    }

    /// The agent slug advertised by a pubkey's profile, if any (and non-empty).
    pub fn resolve_slug_for_pubkey(&self, pubkey: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT slug, host, agent_slug FROM relay_profiles WHERE pubkey=?1",
                params![pubkey],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?
            .map(|(slug, host, agent_slug)| {
                crate::idref::session_handle_from_profile_name(&slug, &host, &agent_slug)
            })
            .filter(|s| !s.is_empty()))
    }
}
