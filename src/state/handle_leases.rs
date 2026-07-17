use super::*;
use std::hash::{Hash, Hasher};

mod allocation;

pub(crate) const HANDLE_LEASE_GRACE_SECS: u64 = 7 * 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HandleAllocation {
    pub(crate) handle: String,
    pub(crate) reclaimed_pubkey: Option<String>,
}

impl Store {
    pub(crate) fn ensure_custom_handle_available(
        &self,
        agent_slug: &str,
        name: &str,
    ) -> Result<()> {
        let handle = custom_handle(agent_slug, name)?;
        let occupied: bool = self.conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM handle_leases AS lease
                 WHERE lease.handle=?1 AND (
                     lease.live=1 OR EXISTS(
                         SELECT 1 FROM sessions AS session
                         WHERE session.pubkey=lease.pubkey AND session.alive=1
                     )
                 )
             )",
            [&handle],
            |row| row.get(0),
        )?;
        if occupied {
            anyhow::bail!("session name {name:?} is already in use as {handle:?}");
        }
        Ok(())
    }

    pub(crate) fn handle_for_pubkey(&self, pubkey: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT handle FROM handle_leases WHERE pubkey=?1",
                [pubkey],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub(crate) fn pubkey_for_handle(&self, handle: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT pubkey FROM handle_leases WHERE handle=?1",
                [handle.trim().trim_start_matches('@')],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub(crate) fn touch_handle_for_pubkey(&self, pubkey: &str, at: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE handle_leases SET last_active_at=?2
             WHERE live=1 AND pubkey=?1",
            params![pubkey, at],
        )?;
        Ok(())
    }

    pub(crate) fn mark_handle_offline_for_pubkey(&self, pubkey: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE handle_leases SET live=0,
                 last_active_at=MAX(last_active_at,
                     COALESCE((SELECT last_seen FROM sessions WHERE pubkey=?1), 0))
             WHERE pubkey=?1",
            [pubkey],
        )?;
        Ok(())
    }

    pub fn session_identity(
        &self,
        pubkey: &str,
    ) -> Result<Option<crate::identity::SessionIdentity>> {
        let Some(session) = self.get_session(pubkey)? else {
            return Ok(None);
        };
        let durable = !self.is_derived_session_pubkey(pubkey)?;
        let handle = match (durable, self.handle_for_pubkey(pubkey)?) {
            (true, _) => session.agent_slug.clone(),
            (false, Some(handle)) => handle,
            (false, None) => anyhow::bail!("derived session {pubkey} has no handle lease"),
        };
        Ok(Some(crate::identity::SessionIdentity::new(
            pubkey.to_string(),
            session.agent_slug,
            handle,
            durable,
        )))
    }
}

pub(super) fn custom_handle(agent_slug: &str, name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("session name must not be empty");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        anyhow::bail!(
            "session name {name:?} must use only ASCII letters, digits, hyphens, or underscores"
        );
    }
    Ok(crate::idref::session_handle(agent_slug, name))
}

pub(super) fn candidates(seed: &str) -> impl Iterator<Item = String> {
    let words = crate::util::CODE_WORDS_A
        .iter()
        .chain(crate::util::CODE_WORDS_B.iter())
        .copied()
        .collect::<Vec<_>>();
    let seed = seed_hash(seed);
    let tier1 = rotated(words.len(), seed).map({
        let words = words.clone();
        move |i| words[i].to_string()
    });
    let tier2_len = words.len() * 1000;
    let tier2 = rotated(tier2_len, seed.rotate_left(17)).map({
        let words = words.clone();
        move |i| format!("{}-{:03}", words[i / 1000], i % 1000)
    });
    let tier3_len = crate::util::CODE_WORDS_A.len() * crate::util::CODE_WORDS_B.len() * 1000;
    let tier3 = rotated(tier3_len, seed.rotate_left(33)).map(|i| {
        let number = i % 1000;
        let pair = i / 1000;
        format!(
            "{}-{}-{number:03}",
            crate::util::CODE_WORDS_A[pair / crate::util::CODE_WORDS_B.len()],
            crate::util::CODE_WORDS_B[pair % crate::util::CODE_WORDS_B.len()]
        )
    });
    tier1.chain(tier2).chain(tier3)
}

fn rotated(len: usize, seed: u64) -> impl Iterator<Item = usize> {
    let start = seed as usize % len;
    (0..len).map(move |offset| (start + offset) % len)
}

fn seed_hash(seed: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
#[path = "handle_leases/tests.rs"]
mod tests;
