//! Atomic public-handle and derived-session signer allocation.

use super::*;
use rusqlite::{Transaction, TransactionBehavior};

impl Store {
    #[cfg(test)]
    pub(crate) fn reserve_handle_for_pubkey(
        &self,
        pubkey: &str,
        agent_slug: &str,
        session_name: Option<&str>,
        now: u64,
    ) -> Result<HandleAllocation> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let allocation = match session_name {
            Some(name) => allocate_custom_handle_in(&tx, pubkey, agent_slug, name, now)?,
            None => allocate_handle_in(&tx, pubkey, agent_slug, now)?,
        };
        tx.commit()?;
        Ok(allocation)
    }

    #[cfg(test)]
    pub(crate) fn allocate_handle(
        &self,
        pubkey: &str,
        agent_slug: &str,
        now: u64,
    ) -> Result<HandleAllocation> {
        self.reserve_handle_for_pubkey(pubkey, agent_slug, None, now)
    }

    #[cfg(test)]
    pub(crate) fn allocate_custom_handle(
        &self,
        pubkey: &str,
        agent_slug: &str,
        name: &str,
        now: u64,
    ) -> Result<HandleAllocation> {
        self.reserve_handle_for_pubkey(pubkey, agent_slug, Some(name), now)
    }

    pub(crate) fn reserve_derived_identity<T>(
        &self,
        agent_slug: &str,
        session_name: Option<&str>,
        now: u64,
        derive: impl FnOnce(&str) -> Result<(T, String)>,
    ) -> Result<(T, String, HandleAllocation)> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let signer_salt = crate::identity::new_session_signer_salt();
        let (value, pubkey) = derive(&signer_salt)?;
        tx.execute(
            "INSERT OR IGNORE INTO session_signers (pubkey, signer_salt) VALUES (?1, ?2)",
            params![pubkey, signer_salt],
        )?;
        let stored_salt: String = tx.query_row(
            "SELECT signer_salt FROM session_signers WHERE pubkey=?1",
            [&pubkey],
            |row| row.get(0),
        )?;
        if stored_salt != signer_salt {
            anyhow::bail!("signer material changed for pubkey {pubkey:?}");
        }
        let allocation = match session_name {
            Some(name) => allocate_custom_handle_in(&tx, &pubkey, agent_slug, name, now)?,
            None => allocate_handle_in(&tx, &pubkey, agent_slug, now)?,
        };
        tx.commit()?;
        Ok((value, pubkey, allocation))
    }
}

fn allocate_handle_in(
    tx: &Transaction<'_>,
    pubkey: &str,
    agent_slug: &str,
    now: u64,
) -> Result<HandleAllocation> {
    if let Some(handle) = lease_for_pubkey(tx, pubkey)? {
        tx.execute(
            "UPDATE handle_leases SET live=1, last_active_at=?2 WHERE pubkey=?1",
            params![pubkey, now],
        )?;
        return Ok(HandleAllocation {
            handle,
            reclaimed_pubkey: None,
        });
    }
    for codename in candidates(pubkey) {
        let handle = crate::idref::session_handle(agent_slug, &codename);
        let occupied = tx
            .query_row(
                "SELECT lease.pubkey,
                        (lease.live=1 OR EXISTS(
                            SELECT 1 FROM sessions AS session
                            WHERE session.pubkey=lease.pubkey AND session.alive=1
                        )),
                        lease.last_active_at
                 FROM handle_leases AS lease WHERE lease.handle=?1",
                [&handle],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
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
        if reclaimed_pubkey.is_some() {
            tx.execute("DELETE FROM handle_leases WHERE handle=?1", [&handle])?;
        }
        tx.execute(
            "INSERT INTO handle_leases
                (handle, pubkey, agent_slug, leased_at, last_active_at, live)
             VALUES (?1, ?2, ?3, ?4, ?4, 1)",
            params![handle, pubkey, agent_slug, now],
        )?;
        return Ok(HandleAllocation {
            handle,
            reclaimed_pubkey,
        });
    }
    anyhow::bail!("session handle space exhausted for agent {agent_slug:?}")
}

fn allocate_custom_handle_in(
    tx: &Transaction<'_>,
    pubkey: &str,
    agent_slug: &str,
    name: &str,
    now: u64,
) -> Result<HandleAllocation> {
    let handle = custom_handle(agent_slug, name)?;
    if let Some(existing) = lease_for_pubkey(tx, pubkey)? {
        if existing != handle {
            anyhow::bail!(
                "session {pubkey:?} already uses {existing:?}, not requested name {name:?}"
            );
        }
        tx.execute(
            "UPDATE handle_leases SET live=1, last_active_at=?2 WHERE pubkey=?1",
            params![pubkey, now],
        )?;
        return Ok(HandleAllocation {
            handle: existing,
            reclaimed_pubkey: None,
        });
    }
    let occupied = tx
        .query_row(
            "SELECT lease.pubkey,
                    (lease.live=1 OR EXISTS(
                        SELECT 1 FROM sessions AS session
                        WHERE session.pubkey=lease.pubkey AND session.alive=1
                    ))
             FROM handle_leases AS lease WHERE lease.handle=?1",
            [&handle],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
        )
        .optional()?;
    let reclaimed_pubkey = occupied.as_ref().map(|(owner, _)| owner.clone());
    match occupied {
        Some((_owner, true)) => {
            anyhow::bail!("session name {name:?} is already in use as {handle:?}")
        }
        Some((owner, false)) => {
            tx.execute("DELETE FROM handle_leases WHERE handle=?1", [&handle])?;
            tracing::debug!(pubkey = %owner, %handle, "reclaiming dead custom handle lease");
        }
        None => {}
    }
    tx.execute(
        "INSERT INTO handle_leases
            (handle, pubkey, agent_slug, leased_at, last_active_at, live)
         VALUES (?1, ?2, ?3, ?4, ?4, 1)",
        params![handle, pubkey, agent_slug, now],
    )?;
    Ok(HandleAllocation {
        handle,
        reclaimed_pubkey,
    })
}

fn lease_for_pubkey(tx: &Transaction<'_>, pubkey: &str) -> Result<Option<String>> {
    tx.query_row(
        "SELECT handle FROM handle_leases WHERE pubkey=?1",
        [pubkey],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_conflict_rolls_back_signer() {
        let store = Store::open_memory().unwrap();
        store
            .allocate_custom_handle("owner", "codex", "named", 1)
            .unwrap();
        let error = store
            .reserve_derived_identity("codex", Some("named"), 2, |_| {
                Ok(((), "new-pk".to_string()))
            })
            .unwrap_err();

        assert!(error.to_string().contains("already in use"));
        assert!(store.session_signer_salt("new-pk").unwrap().is_none());
        assert_eq!(
            store.pubkey_for_handle("named-codex").unwrap().as_deref(),
            Some("owner")
        );
    }
}
