use super::*;
use rusqlite::{Transaction, TransactionBehavior};
use std::hash::{Hash, Hasher};

pub(crate) const HANDLE_LEASE_GRACE_SECS: u64 = 7 * 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HandleAllocation {
    pub(crate) handle: String,
    pub(crate) codename: String,
    pub(crate) reclaimed_pubkey: Option<String>,
}

impl Store {
    pub(super) fn backfill_handle_leases(&self) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO handle_leases
                (handle, pubkey, agent_slug, leased_at, last_active_at, live)
             SELECT codename || '-' || agent_slug, pubkey, agent_slug, created_at,
                    COALESCE((SELECT MAX(last_seen, created_at) FROM sessions
                              WHERE sessions.session_id=identities.session_id), created_at),
                    COALESCE((SELECT alive FROM sessions
                              WHERE sessions.session_id=identities.session_id), alive)
             FROM identities
             WHERE codename<>'' AND agent_slug<>''",
            [],
        )?;
        Ok(())
    }

    pub(crate) fn allocate_handle(
        &self,
        pubkey: &str,
        agent_slug: &str,
        now: u64,
    ) -> Result<HandleAllocation> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        if let Some((handle, codename)) = lease_for_pubkey(&tx, pubkey)? {
            tx.execute(
                "UPDATE handle_leases SET live=1, last_active_at=?2 WHERE pubkey=?1",
                params![pubkey, now],
            )?;
            tx.commit()?;
            return Ok(HandleAllocation {
                handle,
                codename,
                reclaimed_pubkey: None,
            });
        }

        for codename in candidates(pubkey) {
            let handle = crate::idref::session_handle(agent_slug, &codename);
            let occupied = tx
                .query_row(
                    "SELECT pubkey, live, last_active_at FROM handle_leases WHERE handle=?1",
                    [&handle],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, bool>(1)?,
                            row.get::<_, u64>(2)?,
                        ))
                    },
                )
                .optional()?;
            let reclaimed_pubkey = match occupied {
                None => None,
                Some((_owner, true, _)) => continue,
                Some((_owner, false, last_active))
                    if now.saturating_sub(last_active) < HANDLE_LEASE_GRACE_SECS =>
                {
                    continue;
                }
                Some((owner, false, _)) => Some(owner),
            };

            if let Some(old_pubkey) = reclaimed_pubkey.as_deref() {
                tx.execute(
                    "UPDATE identities SET codename='' WHERE pubkey=?1",
                    [old_pubkey],
                )?;
                tx.execute("DELETE FROM handle_leases WHERE handle=?1", [&handle])?;
            }
            tx.execute(
                "INSERT INTO handle_leases
                    (handle, pubkey, agent_slug, leased_at, last_active_at, live)
                 VALUES (?1, ?2, ?3, ?4, ?4, 1)",
                params![handle, pubkey, agent_slug, now],
            )?;
            tx.commit()?;
            return Ok(HandleAllocation {
                handle,
                codename,
                reclaimed_pubkey,
            });
        }
        anyhow::bail!("session handle space exhausted for agent {agent_slug:?}")
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

    pub(crate) fn touch_handle_for_session(&self, session_id: &str, at: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE handle_leases SET last_active_at=?2
             WHERE live=1 AND pubkey=(SELECT pubkey FROM identities WHERE session_id=?1 LIMIT 1)",
            params![session_id, at],
        )?;
        Ok(())
    }

    pub(crate) fn mark_handle_offline_for_session(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE handle_leases SET live=0,
                 last_active_at=MAX(last_active_at,
                     COALESCE((SELECT last_seen FROM sessions WHERE session_id=?1), 0))
             WHERE pubkey=(SELECT pubkey FROM identities WHERE session_id=?1 LIMIT 1)",
            [session_id],
        )?;
        Ok(())
    }
}

fn lease_for_pubkey(tx: &Transaction<'_>, pubkey: &str) -> Result<Option<(String, String)>> {
    let row = tx
        .query_row(
            "SELECT handle, agent_slug FROM handle_leases WHERE pubkey=?1",
            [pubkey],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    Ok(row.map(|(handle, agent_slug)| {
        let suffix = format!("-{agent_slug}");
        let codename = handle.strip_suffix(&suffix).unwrap_or(&handle).to_string();
        (handle, codename)
    }))
}

fn candidates(seed: &str) -> impl Iterator<Item = String> {
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
