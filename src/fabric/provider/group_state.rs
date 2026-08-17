use super::Nip29Provider;
use anyhow::{Context, Result};
use std::time::Duration;

impl Nip29Provider {
    /// Whether any host in scope holds any of `group`'s relay-signed records.
    ///
    /// Read from the WIRE, and deliberately never from `relay_channels`. That
    /// cache also carries the LOCAL row `channel_init` writes for a workspace
    /// root before the group is provisioned, so a cached row is not evidence
    /// that the relay has the group. Provisioning is precisely the decision
    /// that must not be fooled by one — reading a local reservation as relay
    /// truth skips creation and leaves the workspace unprovisioned.
    ///
    /// Only the KIND of each returned record is inspected, so this needs no
    /// per-event host attribution and parses no `p` row.
    pub(in crate::fabric::provider) async fn group_records_exist(
        &self,
        group: &str,
    ) -> Result<bool> {
        let read = self
            .nmp
            .fetch_group_records(group, 30, Duration::from_secs(5))
            .await
            .context("group_records_exist: relay fetch of group records failed")?;
        require_proven_projection(&read, "checking whether the relay hosts the group")?;
        Ok(read.rows.iter().any(|row| {
            matches!(
                row.event.kind.as_u16(),
                nmp_nip29::GROUP_METADATA_KIND
                    | nmp_nip29::GROUP_ADMINS_KIND
                    | nmp_nip29::GROUP_MEMBERS_KIND
            )
        }))
    }

    /// Whether a relay-signed kind:39001 for `group` has been observed at all.
    ///
    /// Distinguishes "the admin list does not name X" from "no admin list has
    /// arrived yet". Acting on the second as if it were the first is how a
    /// daemon fires a repair against state it has never seen.
    pub(in crate::fabric::provider) fn admin_list_observed(&self, group: &str) -> bool {
        self.with_store(|store| {
            store
                .channel_member_sets(group)
                .map(|sets| sets.iter().any(|set| set.role == "admin"))
                .unwrap_or(false)
        })
    }

    /// Fetch the declared parent without collapsing a transport failure into
    /// `None`. Readiness uses this fail-closed surface before verifying the
    /// reciprocal parent metadata.
    pub(crate) async fn try_fetch_group_parent(&self, group: &str) -> Result<Option<String>> {
        use crate::fabric::nip29::wire::KIND_GROUP_METADATA;
        let read = self
            .nmp
            .fetch_group_records(group, 10, Duration::from_secs(5))
            .await
            .context("fetch_group_parent: relay fetch of kind:39000 failed")?;
        require_proven_projection(&read, "reading the relay-authored group parent")?;
        let Some(newest) = read
            .rows
            .iter()
            .map(|row| &row.event)
            .filter(|event| event.kind.as_u16() == KIND_GROUP_METADATA)
            .max_by_key(|event| event.created_at.as_secs())
        else {
            return Ok(None);
        };
        Ok(newest.tags.iter().find_map(|t| {
            let s = t.as_slice();
            if s.first().map(String::as_str) == Some("parent") {
                s.get(1).filter(|parent| !parent.is_empty()).cloned()
            } else {
                None
            }
        }))
    }

    /// Fetch the relay-authored kind:39000 for ONE `group` and materialize it into
    /// `relay_channels` via the single inbound materializer. Returns `true`
    /// only when this NMP-proven read itself contained kind:39000. Existing
    /// rows are usable with durable coverage, but never merely because a cache
    /// row exists after a timeout, disconnect, or acquisition shortfall.
    pub async fn fetch_and_materialize_channel(&self, group: &str) -> Result<bool> {
        use crate::fabric::nip29::wire::KIND_GROUP_METADATA;
        let read = self
            .nmp
            .fetch_group_records(group, 10, Duration::from_secs(5))
            .await
            .context("fetching relay-authored group metadata")?;
        require_proven_projection(&read, "materializing relay-authored group metadata")?;
        let Some(newest) = read
            .rows
            .iter()
            .filter(|row| row.event.kind.as_u16() == KIND_GROUP_METADATA)
            .max_by_key(|row| row.event.created_at.as_secs())
        else {
            return Ok(false);
        };
        self.materialize_bounded_row(
            &format!("bounded-group-metadata:{group}"),
            newest,
            &read.evidence,
        )?;
        Ok(true)
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
