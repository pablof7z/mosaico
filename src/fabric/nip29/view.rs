//! Complete decoded NIP-29 product state derived from active relay demands.
//!
//! Every product read model is derived here, once per replacement. Readers
//! borrow the current level through an `Arc`; no read decodes or rebuilds.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};

use nostr::prelude::Event;
use tokio::sync::watch;

use crate::domain::DomainEvent;
use crate::group_projection::GroupProjection;
use crate::group_snapshot::{decode_snapshots, GroupAvailability, GroupSnapshot};
use crate::state::{Profile, RelayEvent, Status};

#[path = "view/decode.rs"]
mod decode;

use decode::{decode_message, decode_reaction};
pub(crate) use decode::{MessageProjection, ReactionProjection};

#[derive(Clone, Default)]
struct GroupScope {
    ids: BTreeSet<String>,
    subjects: BTreeSet<String>,
}

/// One complete decoded current level.
#[derive(Default)]
pub(crate) struct Nip29View {
    content: Vec<Event>,
    group_snapshots: Vec<GroupSnapshot>,
    groups: GroupProjection,
    profiles: Vec<Profile>,
    profile_index: HashMap<String, usize>,
    statuses: Vec<Status>,
    events: Vec<RelayEvent>,
    event_index: HashMap<String, usize>,
    messages: Vec<MessageProjection>,
    message_index: HashMap<String, usize>,
    reactions: Vec<ReactionProjection>,
}

impl Nip29View {
    fn decode(events: Vec<Event>, scope: &GroupScope) -> Self {
        let group_events = events
            .iter()
            .filter(|event| matches!(event.kind.as_u16(), 39_000..=39_002))
            .cloned()
            .collect::<Vec<_>>();
        let group_snapshots =
            decode_snapshots(&group_events, &BTreeSet::new(), GroupAvailability::Ready)
                .into_iter()
                .filter(|snapshot| group_matches(snapshot, scope))
                .collect::<Vec<_>>();
        let groups = GroupProjection::new(&group_snapshots);
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
            .collect::<Vec<_>>();
        let statuses = content.iter().flat_map(statuses_from_event).collect();
        let relay_events = content.iter().map(relay_event).collect();
        Self::assemble(content, group_snapshots, groups, profiles, statuses, relay_events)
    }

    /// A level from already-projected parts, for tests that install rows
    /// without signing relay events.
    #[cfg(test)]
    pub(crate) fn from_parts(
        groups: GroupProjection,
        profiles: Vec<Profile>,
        statuses: Vec<Status>,
        events: Vec<RelayEvent>,
    ) -> Self {
        Self::assemble(Vec::new(), Vec::new(), groups, profiles, statuses, events)
    }

    fn assemble(
        content: Vec<Event>,
        group_snapshots: Vec<GroupSnapshot>,
        groups: GroupProjection,
        profiles: Vec<Profile>,
        mut statuses: Vec<Status>,
        events: Vec<RelayEvent>,
    ) -> Self {
        let profile_index = first_index(profiles.iter().map(|profile| &profile.pubkey));
        // A status that omitted its `slug` tag takes the slug from its kind:0
        // profile in this same level.
        for status in statuses.iter_mut().filter(|status| status.slug.is_empty()) {
            if let Some(&index) = profile_index.get(&status.pubkey) {
                status.slug = profiles[index].slug.clone();
            }
        }
        let event_index = first_index(events.iter().map(|event| &event.id));
        let messages = events.iter().filter_map(decode_message).collect::<Vec<_>>();
        let message_index = first_index(messages.iter().map(|row| &row.message.message_id));
        let reactions = events.iter().filter_map(decode_reaction).collect();
        Self {
            content,
            group_snapshots,
            groups,
            profiles,
            profile_index,
            statuses,
            events,
            event_index,
            messages,
            message_index,
            reactions,
        }
    }

    /// Signed content events, for consumers that act on the events themselves.
    pub(crate) fn content(&self) -> &[Event] {
        &self.content
    }

    pub(crate) fn group_snapshots(&self) -> &[GroupSnapshot] {
        &self.group_snapshots
    }

    pub(crate) fn groups(&self) -> &GroupProjection {
        &self.groups
    }

    pub(crate) fn profiles(&self) -> &[Profile] {
        &self.profiles
    }

    pub(crate) fn profile(&self, pubkey: &str) -> Option<&Profile> {
        self.profile_index
            .get(pubkey)
            .map(|&index| &self.profiles[index])
    }

    pub(crate) fn statuses(&self) -> &[Status] {
        &self.statuses
    }

    pub(crate) fn events(&self) -> &[RelayEvent] {
        &self.events
    }

    pub(crate) fn event(&self, id: &str) -> Option<&RelayEvent> {
        self.event_index.get(id).map(|&index| &self.events[index])
    }

    pub(crate) fn messages(&self) -> &[MessageProjection] {
        &self.messages
    }

    pub(crate) fn message(&self, id: &str) -> Option<&MessageProjection> {
        self.message_index.get(id).map(|&index| &self.messages[index])
    }

    pub(crate) fn reactions(&self) -> &[ReactionProjection] {
        &self.reactions
    }
}

/// Atomically replaced complete product level. Revisions are wake-only.
pub(crate) struct Nip29ViewStore {
    current: RwLock<Arc<Nip29View>>,
    group_scope: RwLock<GroupScope>,
    changed: watch::Sender<u64>,
}

impl Default for Nip29ViewStore {
    fn default() -> Self {
        let (changed, _) = watch::channel(0);
        Self {
            current: RwLock::new(Arc::default()),
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
        let next = Arc::new(Nip29View::decode(events, &scope));
        *self
            .current
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = next;
        self.changed
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    /// The current level. Cloning the `Arc` is the whole cost of a read.
    pub(crate) fn current(&self) -> Arc<Nip29View> {
        Arc::clone(
            &self
                .current
                .read()
                .unwrap_or_else(|poison| poison.into_inner()),
        )
    }

    pub(crate) fn subscribe_changes(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    pub(crate) async fn wait_for_group_metadata(&self, group: &str) -> anyhow::Result<()> {
        let mut changes = self.subscribe_changes();
        loop {
            if self
                .current()
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

/// Position of the first row for each key, matching a front-to-back `find`.
fn first_index<'a>(keys: impl Iterator<Item = &'a String>) -> HashMap<String, usize> {
    let mut index = HashMap::new();
    for (position, key) in keys.enumerate() {
        index.entry(key.clone()).or_insert(position);
    }
    index
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

fn statuses_from_event(event: &Event) -> Vec<Status> {
    let Some(DomainEvent::Status(status)) =
        crate::fabric::nip29::wire::Nip29WireCodec.decode_event(event)
    else {
        return Vec::new();
    };
    let updated_at = event.created_at.as_secs();
    status
        .channels
        .iter()
        .map(|channel_h| Status {
            pubkey: status.agent.pubkey.clone(),
            channel_h: channel_h.clone(),
            slug: status.agent.slug.clone(),
            title: status.title.clone(),
            activity: status.activity.clone(),
            workspace: status.workspace.clone(),
            branch: status.branch.clone(),
            state: status.state,
            state_since: status.state_since,
            last_seen: updated_at,
            updated_at,
            expiration: status.expires_at.unwrap_or(0),
        })
        .collect()
}

fn relay_event(event: &Event) -> RelayEvent {
    RelayEvent {
        id: event.id.to_hex(),
        kind: event.kind.as_u16() as u32,
        pubkey: event.pubkey.to_hex(),
        created_at: event.created_at.as_secs(),
        channel_h: crate::fabric::nip29::nostr_tag(event, "h")
            .unwrap_or("")
            .to_string(),
        d_tag: crate::fabric::nip29::nostr_tag(event, "d")
            .unwrap_or("")
            .to_string(),
        content: event.content.clone(),
        tags_json: serde_json::to_string(
            &event
                .tags
                .iter()
                .map(|tag| tag.as_slice().to_vec())
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".to_string()),
    }
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
