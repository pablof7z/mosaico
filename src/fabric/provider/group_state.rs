use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::{Context, Result};
use nostr::prelude::{Filter, Kind};

use super::Nip29Provider;
use crate::group_snapshot::{decode_snapshots, GroupAvailability, GroupSnapshot};
use crate::nostr_runtime::GROUP_RECORD_LIMIT;

const GROUP_READ_DEMAND: &str = "mosaico-group-read";

impl Nip29Provider {
    pub(crate) fn current_group_snapshot(&self, group: &str) -> Option<GroupSnapshot> {
        self.view
            .current()
            .group_snapshots()
            .iter()
            .find(|snapshot| {
                snapshot.id == group
                    && (snapshot.metadata.is_some()
                        || !snapshot.admins.is_empty()
                        || !snapshot.members.is_empty())
            })
            .cloned()
    }

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

    pub(crate) async fn group_snapshot(&self, group: &str) -> Result<GroupSnapshot> {
        if let Some(snapshot) = self.current_group_snapshot(group) {
            return Ok(snapshot);
        }
        let _serial = self.group_read.lock().await;
        let filter = Filter::new()
            .kinds([
                Kind::Custom(39_000),
                Kind::Custom(39_001),
                Kind::Custom(39_002),
            ])
            .identifier(group)
            .limit(GROUP_RECORD_LIMIT.min(3));
        self.runtime
            .replace_demand(GROUP_READ_DEMAND, vec![filter])
            .await
            .with_context(|| format!("opening group read for {group}"))?;
        let result = async {
            self.runtime
                .wait_for_eose(GROUP_READ_DEMAND, Duration::from_secs(5))
                .await?;
            let events = self.runtime.snapshot().await?.events;
            Ok(decode_snapshots(
                &events,
                &BTreeSet::from([group.to_string()]),
                GroupAvailability::Ready,
            )
            .into_iter()
            .next()
            .unwrap_or_else(|| empty_snapshot(group)))
        }
        .await;
        let close = self.runtime.close_demand(GROUP_READ_DEMAND).await;
        close.context("closing group read")?;
        result
    }

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

fn empty_snapshot(group: &str) -> GroupSnapshot {
    GroupSnapshot {
        id: group.to_string(),
        metadata: None,
        admins: Vec::new(),
        members: Vec::new(),
        availability: GroupAvailability::Ready,
    }
}
