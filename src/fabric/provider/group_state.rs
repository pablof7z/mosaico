use super::Nip29Provider;
use anyhow::{Context, Result};
use std::time::Duration;

use crate::group_snapshot::{GroupAvailability, GroupSnapshot};

impl Nip29Provider {
    /// The current record-bearing value from the daemon's retained group
    /// observation. An id-only Acquiring seed is not treated as group state.
    pub(crate) fn current_group_snapshot(&self, group: &str) -> Option<GroupSnapshot> {
        self.nmp
            .views()
            .group_snapshots()
            .into_iter()
            .find(|snapshot| {
                snapshot.id == group
                    && (snapshot.metadata.is_some()
                        || !snapshot.admins.is_empty()
                        || !snapshot.members.is_empty())
            })
    }

    /// Fetch the declared parent without collapsing a transport failure into
    /// `None`. Readiness uses this fail-closed surface before verifying the
    /// reciprocal parent metadata.
    pub(crate) async fn try_fetch_group_parent(&self, group: &str) -> Result<Option<String>> {
        let snapshot = self.group_snapshot(group).await?;
        let Some(metadata) = snapshot.metadata else {
            return Ok(None);
        };
        Ok(metadata.tags.iter().find_map(|row| {
            if row.first().map(String::as_str) == Some("parent") {
                row.get(1).filter(|parent| !parent.is_empty()).cloned()
            } else {
                None
            }
        }))
    }

    /// Read one complete group value from fava. Nothing is copied into SQLite.
    pub(crate) async fn group_snapshot(&self, group: &str) -> Result<GroupSnapshot> {
        #[cfg(test)]
        if let Some(scripted) = self.nmp.take_scripted_group_snapshot(group) {
            return scripted.context("reading scripted group snapshot");
        }

        // The daemon already owns one retained observation. A group with
        // delivered records is read from that value directly rather than
        // opening another query and waiting for the same rows to be projected
        // again. An id-only Acquiring seed with no records is not an answer:
        // the bounded group-scoped observation below is still needed to
        // distinguish a genuinely absent group before creation.
        if let Some(snapshot) = self.current_group_snapshot(group) {
            return Ok(snapshot);
        }

        let mut observation = self
            .nmp
            .open_one_group_records_observation(group)
            .await
            .with_context(|| format!("opening group observation for {group}"))?;
        tokio::time::timeout(Duration::from_secs(5), async {
            use crate::group_snapshot::decode_snapshots;
            use fava::ObservationClosed;
            use std::collections::BTreeSet;

            loop {
                let fava_snapshot = match observation.changed().await {
                    Ok(s) => s,
                    Err(ObservationClosed) => {
                        anyhow::bail!("group observation closed before acquisition settled")
                    }
                };
                let av = crate::group_snapshot::availability_from_evidence(&fava_snapshot.evidence);
                let snapshots = decode_snapshots(&fava_snapshot, &BTreeSet::new(), av);
                let Some(snapshot) = snapshots.into_iter().find(|s| s.id == group) else {
                    // No records yet for this group.
                    match av {
                        GroupAvailability::Ready | GroupAvailability::CachedOnly => {
                            // Relay settled with no records: group does not exist.
                            return Ok(GroupSnapshot {
                                id: group.to_string(),
                                metadata: None,
                                admins: Vec::new(),
                                members: Vec::new(),
                                availability: av,
                            });
                        }
                        GroupAvailability::Acquiring | GroupAvailability::SourceUnavailable => {
                            continue;
                        }
                    }
                };
                match snapshot.availability {
                    GroupAvailability::Acquiring | GroupAvailability::SourceUnavailable => {
                        continue;
                    }
                    GroupAvailability::CachedOnly | GroupAvailability::Ready => {
                        return Ok(snapshot);
                    }
                }
            }
        })
        .await
        .with_context(|| format!("timed out reading group snapshot for {group}"))?
    }

    /// Read the current relay-authored metadata as a product channel value.
    /// The value is returned to the caller; it is never materialized.
    pub async fn fetch_channel(&self, group: &str) -> Result<Option<crate::state::Channel>> {
        let snapshot = self.group_snapshot(group).await?;
        let Some(metadata) = snapshot.metadata else {
            return Ok(None);
        };
        let parent = metadata
            .tags
            .iter()
            .find(|row| row.first().map(String::as_str) == Some("parent"))
            .and_then(|row| row.get(1))
            .cloned()
            .unwrap_or_default();
        let as_of = metadata.as_of.as_secs();
        Ok(Some(crate::state::Channel {
            channel_h: snapshot.id,
            name: metadata.name.unwrap_or_default(),
            about: metadata.about.unwrap_or_default(),
            parent,
            created_at: as_of,
            updated_at: as_of,
        }))
    }
}

pub(super) fn require_proven_projection(
    read: &crate::nmp_host::read::BoundedRead,
    action: &str,
) -> Result<()> {
    if matches!(
        read.termination,
        crate::nmp_host::read::BoundedReadTermination::RelaySettled
    ) {
        return Ok(());
    }
    anyhow::bail!(
        "{action} ended as {:?}; relay did not settle before the deadline",
        read.termination,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nmp_host::read::{BoundedRead, BoundedReadTermination};

    fn read(termination: BoundedReadTermination) -> BoundedRead {
        BoundedRead {
            rows: Vec::new(),
            termination,
        }
    }

    #[test]
    fn relay_settled_proves_projection_but_timeout_does_not() {
        assert!(require_proven_projection(
            &read(BoundedReadTermination::RelaySettled),
            "projection"
        )
        .is_ok());
        assert!(
            require_proven_projection(&read(BoundedReadTermination::TimedOut), "projection")
                .is_err()
        );
    }
}
