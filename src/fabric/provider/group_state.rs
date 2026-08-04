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
        let records = self
            .nmp
            .fetch_group_records(group, 30, Duration::from_secs(5))
            .await
            .context("group_records_exist: relay fetch of group records failed")?;
        Ok(records.iter().any(|event| {
            matches!(
                event.kind.as_u16(),
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

    /// The `parent` group id declared in `group`'s relay-authored kind:39000 metadata.
    pub async fn fetch_group_parent(&self, group: &str) -> Option<String> {
        match self.try_fetch_group_parent(group).await {
            Ok(parent) => parent,
            Err(e) => {
                tracing::error!(
                    group,
                    error = %format!("{e:#}"),
                    "fetch_group_parent: relay fetch failed — could not determine parent"
                );
                None
            }
        }
    }

    /// Fetch the declared parent without collapsing a transport failure into
    /// `None`. Readiness uses this fail-closed surface before verifying the
    /// reciprocal parent metadata.
    pub(in crate::fabric::provider) async fn try_fetch_group_parent(
        &self,
        group: &str,
    ) -> Result<Option<String>> {
        use crate::fabric::nip29::wire::KIND_GROUP_METADATA;
        let evs = self
            .nmp
            .fetch_group_records(group, 10, Duration::from_secs(5))
            .await
            .context("fetch_group_parent: relay fetch of kind:39000 failed")?;
        let Some(newest) = evs
            .iter()
            .filter(|e| e.kind.as_u16() == KIND_GROUP_METADATA)
            .max_by_key(|e| e.created_at.as_secs())
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
    /// `relay_channels` via the single inbound materializer. Returns `true` once a
    /// row for `group` exists in the cache. This is how a just-created group enters
    /// the cache: by reading back the relay's own metadata — never by a local
    /// optimistic write.
    pub async fn fetch_and_materialize_channel(&self, group: &str) -> bool {
        use crate::fabric::nip29::materializer::Nip29Materializer;
        use crate::fabric::nip29::wire::KIND_GROUP_METADATA;
        let evs = match self
            .nmp
            .fetch_group_records(group, 10, Duration::from_secs(5))
            .await
        {
            Ok(evs) => evs,
            Err(e) => {
                // Relay fetch failed: surface it loudly. We fall through to the
                // existing-cache check rather than fabricating a row.
                tracing::error!(
                    group,
                    error = %format!("{e:#}"),
                    "fetch_and_materialize_channel: relay fetch of kind:39000 failed — cannot materialize"
                );
                Vec::new()
            }
        };
        if let Some(newest) = evs
            .iter()
            .filter(|e| e.kind.as_u16() == KIND_GROUP_METADATA)
            .max_by_key(|e| e.created_at.as_secs())
        {
            self.with_store(|s| Nip29Materializer::materialize_channel(s, newest));
        }
        self.with_store(|s| s.get_channel(group).ok().flatten().is_some())
    }

    /// Fetch all kind:39000 events from the relay and materialize them into the
    /// `relay_channels` cache via the single inbound materializer.
    pub async fn refresh_root_channels(&self) -> Result<()> {
        use crate::fabric::nip29::materializer::Nip29Materializer;
        let events = self
            .nmp
            .fetch_all_group_metadata(200, Duration::from_secs(5))
            .await
            .context("refresh_root_channels: relay fetch of kind:39000 list failed")?;
        for ev in &events {
            self.with_store(|s| Nip29Materializer::materialize_channel(s, ev));
        }
        Ok(())
    }
}
