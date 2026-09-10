//! Complete decoded NIP-29 product state derived from active relay demands.

use std::collections::BTreeSet;
use std::sync::RwLock;

use nostr::prelude::Event;
use tokio::sync::watch;

use crate::domain::DomainEvent;
use crate::group_snapshot::{decode_snapshots, GroupAvailability, GroupSnapshot};
use crate::state::Profile;

#[derive(Clone, Default)]
struct GroupScope {
    ids: BTreeSet<String>,
    subjects: BTreeSet<String>,
}

#[derive(Clone, Default)]
struct Nip29View {
    content: Vec<Event>,
    groups: Vec<GroupSnapshot>,
    profiles: Vec<Profile>,
}

/// Atomically replaced complete product level. Revisions are wake-only.
pub(crate) struct Nip29ViewStore {
    current: RwLock<Nip29View>,
    group_scope: RwLock<GroupScope>,
    changed: watch::Sender<u64>,
}

impl Default for Nip29ViewStore {
    fn default() -> Self {
        let (changed, _) = watch::channel(0);
        Self {
            current: RwLock::new(Nip29View::default()),
            group_scope: RwLock::new(GroupScope::default()),
            changed,
        }
    }
}

impl Nip29ViewStore {
    pub(crate) fn replace_group_scope(&self, ids: BTreeSet<String>, subjects: BTreeSet<String>) {
        *self
            .group_scope
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = GroupScope { ids, subjects };
    }

    pub(crate) fn replace(&self, events: Vec<Event>) {
        let scope = self
            .group_scope
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        let group_events = events
            .iter()
            .filter(|event| matches!(event.kind.as_u16(), 39_000..=39_002))
            .cloned()
            .collect::<Vec<_>>();
        let groups = decode_snapshots(&group_events, &BTreeSet::new(), GroupAvailability::Ready)
            .into_iter()
            .filter(|snapshot| group_matches(snapshot, &scope))
            .collect();
        let profiles = events
            .iter()
            .filter_map(profile_from_event)
            .collect::<Vec<_>>();
        let content = events
            .into_iter()
            .filter(|event| {
                !matches!(
                    event.kind,
                    nostr::prelude::Kind::Metadata | nostr::prelude::Kind::EventDeletion
                )
            })
            .filter(|event| !matches!(event.kind.as_u16(), 39_000..=39_002))
            .collect();
        *self
            .current
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = Nip29View {
            content,
            groups,
            profiles,
        };
        self.changed
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    pub(crate) fn content_events(&self) -> Vec<Event> {
        self.current
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .content
            .clone()
    }

    pub(crate) fn group_snapshots(&self) -> Vec<GroupSnapshot> {
        self.current
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .groups
            .clone()
    }

    pub(crate) fn profiles(&self) -> Vec<Profile> {
        self.current
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .profiles
            .clone()
    }

    pub(crate) fn subscribe_changes(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    pub(crate) async fn wait_for_group_metadata(&self, group: &str) -> anyhow::Result<()> {
        let mut changes = self.subscribe_changes();
        loop {
            if self
                .group_snapshots()
                .iter()
                .any(|snapshot| snapshot.id == group && snapshot.metadata.is_some())
            {
                return Ok(());
            }
            changes
                .changed()
                .await
                .map_err(|_| anyhow::anyhow!("NIP-29 view closed before {group} was delivered"))?;
        }
    }
}

fn group_matches(snapshot: &GroupSnapshot, scope: &GroupScope) -> bool {
    scope.ids.contains(&snapshot.id)
        || snapshot
            .admins
            .iter()
            .any(|admin| scope.subjects.contains(&admin.pubkey.to_hex()))
        || snapshot
            .members
            .iter()
            .any(|member| scope.subjects.contains(&member.pubkey.to_hex()))
}

fn profile_from_event(event: &Event) -> Option<Profile> {
    let DomainEvent::Profile(profile) =
        crate::fabric::nip29::wire::Nip29WireCodec.decode_event(event)?
    else {
        return None;
    };
    Some(Profile {
        pubkey: profile.agent.pubkey,
        name: profile.agent.slug.clone(),
        slug: profile.agent.slug,
        agent_slug: profile.agent_slug,
        host: profile.host,
        is_backend: profile.is_backend,
        agents: profile.agents,
        workspaces: profile.workspaces,
        updated_at: event.created_at.as_secs(),
    })
}

#[cfg(test)]
#[path = "view/tests.rs"]
mod tests;
