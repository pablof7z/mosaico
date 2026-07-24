use super::Nip29Provider;
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

impl Nip29Provider {
    /// Fetch the relay's live state for `group`: `(exists, roles, members)`.
    /// A transport failure remains distinct from genuine relay absence.
    pub(crate) async fn fetch_group_state(
        &self,
        group: &str,
    ) -> Result<(bool, HashMap<String, String>, HashSet<String>)> {
        use crate::fabric::nip29::wire::{
            KIND_GROUP_ADMINS, KIND_GROUP_MEMBERS, KIND_GROUP_METADATA,
        };
        let filter = crate::nmp_host::read::filter(
            &[KIND_GROUP_METADATA, KIND_GROUP_ADMINS, KIND_GROUP_MEMBERS],
            &[],
            &[('d', group.to_string())],
        )?;
        let state_evs = self
            .nmp
            .fetch_group(filter, 30, Duration::from_secs(5))
            .await
            .context("fetch_group_state: relay fetch of group state failed")?;

        let newest = |k: u16| {
            state_evs
                .iter()
                .filter(|e| e.kind.as_u16() == k)
                .max_by_key(|e| e.created_at.as_secs())
        };
        let group_exists = newest(KIND_GROUP_METADATA).is_some()
            || newest(KIND_GROUP_ADMINS).is_some()
            || newest(KIND_GROUP_MEMBERS).is_some();

        let mut roles: HashMap<String, String> = HashMap::new();
        if let Some(ev) = newest(KIND_GROUP_ADMINS) {
            for t in ev.tags.iter() {
                let s = t.as_slice();
                if s.first().map(String::as_str) == Some("p") {
                    if let Some(pk) = s.get(1) {
                        roles.insert(
                            pk.clone(),
                            s.get(2).cloned().unwrap_or_else(|| "member".to_string()),
                        );
                    }
                }
            }
        }

        let mut members: HashSet<String> = HashSet::new();
        if let Some(ev) = newest(KIND_GROUP_MEMBERS) {
            for t in ev.tags.iter() {
                let s = t.as_slice();
                if s.first().map(String::as_str) == Some("p") {
                    if let Some(pk) = s.get(1) {
                        members.insert(pk.clone());
                    }
                }
            }
        }
        Ok((group_exists, roles, members))
    }

    /// Convenience: just the role map (kind:39001) for `group`.
    pub(crate) async fn fetch_group_roles(&self, group: &str) -> Result<HashMap<String, String>> {
        Ok(self.fetch_group_state(group).await?.1)
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
        let filter = crate::nmp_host::read::filter(
            &[KIND_GROUP_METADATA],
            &[],
            &[('d', group.to_string())],
        )?;
        let evs = self
            .nmp
            .fetch_group(filter, 10, Duration::from_secs(5))
            .await
            .context("fetch_group_parent: relay fetch of kind:39000 failed")?;
        let Some(newest) = evs.iter().max_by_key(|e| e.created_at.as_secs()) else {
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
        let filter =
            crate::nmp_host::read::filter(&[KIND_GROUP_METADATA], &[], &[('d', group.to_string())])
                .expect("static NMP group metadata filter");
        let evs = match self
            .nmp
            .fetch_group(filter, 10, Duration::from_secs(5))
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
        if let Some(newest) = evs.iter().max_by_key(|e| e.created_at.as_secs()) {
            self.with_store(|s| Nip29Materializer::materialize_channel(s, newest));
        }
        self.with_store(|s| s.get_channel(group).ok().flatten().is_some())
    }

    /// Fetch all kind:39000 events from the relay and materialize them into the
    /// `relay_channels` cache via the single inbound materializer.
    pub async fn refresh_root_channels(&self) -> Result<()> {
        use crate::fabric::nip29::materializer::Nip29Materializer;
        let filter = crate::nmp_host::read::filter(&[39000], &[], &[])?;
        let events = self
            .nmp
            .fetch_group(filter, 200, Duration::from_secs(5))
            .await
            .context("refresh_root_channels: relay fetch of kind:39000 list failed")?;
        for ev in &events {
            self.with_store(|s| Nip29Materializer::materialize_channel(s, ev));
        }
        Ok(())
    }
}
