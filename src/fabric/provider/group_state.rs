use super::Nip29Provider;
use anyhow::{Context, Result};
use std::time::Duration;

impl Nip29Provider {
    /// The current record-bearing value from the daemon's retained NMP group
    /// observation. An id-only Acquiring seed is not treated as group state.
    pub(crate) fn current_group_snapshot(&self, group: &str) -> Option<nmp::nip29::GroupSnapshot> {
        self.nmp
            .views()
            .group_snapshots()
            .into_iter()
            .find(|snapshot| {
                snapshot.id == group
                    && (snapshot.metadata.is_some()
                        || !snapshot.admins.is_empty()
                        || !snapshot.members.is_empty()
                        || !snapshot.per_host.is_empty())
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

    /// Read one complete group value from NMP. Nothing is copied into SQLite.
    pub(crate) async fn group_snapshot(&self, group: &str) -> Result<nmp::nip29::GroupSnapshot> {
        use nmp::nip29::GroupAvailability;

        #[cfg(test)]
        if let Some(scripted) = self.nmp.take_scripted_group_snapshot(group) {
            return scripted.context("reading scripted NMP group snapshot");
        }

        // The daemon already owns one retained GroupObservation. A group with
        // delivered records must be read from that NMP value directly rather
        // than opening another query and waiting for the same rows to be
        // projected again. An id-only Acquiring seed with no records is not an
        // answer: the bounded group-scoped observation below is still needed
        // to distinguish a genuinely absent group before creation.
        if let Some(snapshot) = self.current_group_snapshot(group) {
            return Ok(snapshot);
        }

        let observation = self
            .nmp
            .observe_one_group_records(group)
            .with_context(|| format!("opening NMP group observation for {group}"))?;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let delivered = observation
                    .next()
                    .await
                    .map_err(|error| anyhow::anyhow!("reading NMP group snapshot: {error}"))?
                    .context("NMP group observation ended before acquisition settled")?;
                let snapshot = delivered
                    .into_iter()
                    .next()
                    .context("NMP group observation delivered no group snapshot")?;
                match snapshot.availability {
                    // The daemon accepts local RPCs while its NMP transport is
                    // still warming. Both states can therefore be the first
                    // frame of a live observation and later become Ready. This
                    // bounded accessor waits on that same observation; it does
                    // not reopen or poll another read.
                    GroupAvailability::Acquiring | GroupAvailability::SourceUnavailable => {
                        continue;
                    }
                    GroupAvailability::CachedOnly | GroupAvailability::Ready => {
                        return Ok(snapshot)
                    }
                }
            }
        })
        .await
        .with_context(|| format!("timed out reading NMP group snapshot for {group}"))?
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
            | crate::nmp_host::read::BoundedReadTermination::CoverageProven
    ) {
        return Ok(());
    }
    anyhow::bail!(
        "{action} ended as {:?}; acquisition evidence: {:?}",
        read.termination,
        read.evidence
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nmp_host::read::{BoundedRead, BoundedReadTermination};

    fn read(termination: BoundedReadTermination) -> BoundedRead {
        BoundedRead {
            rows: Vec::new(),
            evidence: Vec::new(),
            termination,
        }
    }

    #[test]
    fn durable_coverage_is_projection_evidence_but_timeout_is_not() {
        assert!(require_proven_projection(
            &read(BoundedReadTermination::RelaySettled),
            "projection"
        )
        .is_ok());
        assert!(require_proven_projection(
            &read(BoundedReadTermination::CoverageProven),
            "projection"
        )
        .is_ok());
        assert!(
            require_proven_projection(&read(BoundedReadTermination::TimedOut), "projection")
                .is_err()
        );
    }
}
