//! Local group-record vocabulary, replacing `nmp::nip29` types.
//!
//! These types mirror the NMP shape so callers need only import-path changes.
//! They are built from fava `QuerySnapshot` rows decoded via `fava_simple_groups`.

use std::collections::BTreeSet;

use fava_simple_groups::{SimpleGroupAdmins, SimpleGroupMembers};
use nostr::event::Event;
use nostr::key::PublicKey;

// ---------------------------------------------------------------- availability

/// Whether relay-authoritative group records are currently available.
///
/// Mirrors `nmp::nip29::GroupAvailability` so call sites need only re-import.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupAvailability {
    /// Relay-live records arrived; the snapshot reflects current relay state.
    Ready,
    /// Only cached records are available; no live relay delivery yet.
    CachedOnly,
    /// Relay demand is being established; no delivery yet.
    Acquiring,
    /// No relay delivered any records and none is expected without intervention.
    SourceUnavailable,
}

// ------------------------------------------------------------------ subjects

/// One relay-signed admin or member entry.
///
/// Mirrors `nmp::nip29::ListedSubject`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedSubject {
    pub pubkey: PublicKey,
    pub roles: Vec<String>,
}

impl ListedSubject {
    pub fn new(pubkey: PublicKey, roles: Vec<String>) -> Self {
        Self { pubkey, roles }
    }
}

// ------------------------------------------------------------------ metadata

/// Relay-signed kind:39000 metadata for one group.
///
/// Mirrors `nmp::nip29::GroupMetadata`.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupMetadata {
    pub name: Option<String>,
    pub about: Option<String>,
    /// Raw tag arrays from the kind:39000 event (for "parent", "picture", etc.)
    pub tags: Vec<Vec<String>>,
    /// Creation/update time of the metadata event (seconds since epoch).
    pub as_of: std::time::Duration,
}

impl GroupMetadata {
    fn from_metadata_event(event: &Event) -> Self {
        let name = tag_value(event, "name").map(str::to_owned);
        let about = tag_value(event, "about").map(str::to_owned);
        let tags: Vec<Vec<String>> = event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect();
        let as_of = std::time::Duration::from_secs(event.created_at.as_secs());
        Self { name, about, tags, as_of }
    }
}

fn tag_value<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|t| {
        let s = t.as_slice();
        if s.first().map(String::as_str) == Some(name) {
            s.get(1).map(String::as_str)
        } else {
            None
        }
    })
}

// ------------------------------------------------------------------ snapshot

/// Complete relay-signed state for one group at one instant.
///
/// Mirrors `nmp::nip29::GroupSnapshot`. The `id` is the `d` tag value.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupSnapshot {
    pub id: String,
    pub metadata: Option<GroupMetadata>,
    pub admins: Vec<ListedSubject>,
    pub members: Vec<ListedSubject>,
    pub availability: GroupAvailability,
}

impl GroupSnapshot {
    /// Builder-style setter for tests.
    #[cfg(test)]
    pub fn availability(mut self, av: GroupAvailability) -> Self {
        self.availability = av;
        self
    }
}

// ------------------------------------------------------------ decode helpers

/// Decode a fava `QuerySnapshot` into `Vec<GroupSnapshot>`.
///
/// `group_ids` restricts which `d` values are included (empty = all).
/// `default_av` is used when relay EOSE is not yet complete.
pub(crate) fn decode_snapshots(
    snapshot: &fava::QuerySnapshot,
    group_ids: &BTreeSet<String>,
    default_av: GroupAvailability,
) -> Vec<GroupSnapshot> {
    use fava::EventValue;
    use std::collections::BTreeMap;

    let mut metadata_map: BTreeMap<String, Event> = BTreeMap::new();
    let mut admins_map: BTreeMap<String, Event> = BTreeMap::new();
    let mut members_map: BTreeMap<String, Event> = BTreeMap::new();

    for record in snapshot.events.iter() {
        let event = match record.event() {
            EventValue::Signed(e) => e,
            EventValue::Unsigned(_) => continue,
        };
        let kind = event.kind.as_u16();
        let d = match d_tag(event) {
            Some(d) => d.to_string(),
            None => continue,
        };
        if !group_ids.is_empty() && !group_ids.contains(&d) {
            continue;
        }
        match kind {
            39_000 => insert_latest(&mut metadata_map, d, event.clone()),
            39_001 => insert_latest(&mut admins_map, d, event.clone()),
            39_002 => insert_latest(&mut members_map, d, event.clone()),
            _ => {}
        }
    }

    let mut all_ids: BTreeSet<String> = BTreeSet::new();
    all_ids.extend(metadata_map.keys().cloned());
    all_ids.extend(admins_map.keys().cloned());
    all_ids.extend(members_map.keys().cloned());
    if !group_ids.is_empty() {
        all_ids.retain(|id| group_ids.contains(id));
    }

    let av = if snapshot.evidence.all_relays_stored_events_complete() {
        GroupAvailability::Ready
    } else {
        default_av
    };

    all_ids
        .into_iter()
        .map(|id| {
            let metadata = metadata_map.get(&id).map(GroupMetadata::from_metadata_event);
            let admins = decode_admins(admins_map.get(&id));
            let members = decode_members(members_map.get(&id));
            GroupSnapshot { id, metadata, admins, members, availability: av }
        })
        .collect()
}

/// Classify availability from fava query evidence.
pub(crate) fn availability_from_evidence(
    evidence: &fava_query::QueryEvidence,
) -> GroupAvailability {
    if evidence.all_relays_stored_events_complete() {
        GroupAvailability::Ready
    } else if evidence.relays.is_empty() {
        GroupAvailability::CachedOnly
    } else {
        GroupAvailability::Acquiring
    }
}

fn insert_latest(
    map: &mut std::collections::BTreeMap<String, Event>,
    key: String,
    event: Event,
) {
    map.entry(key)
        .and_modify(|existing| {
            if event.created_at > existing.created_at {
                *existing = event.clone();
            }
        })
        .or_insert(event);
}

fn decode_admins(event: Option<&Event>) -> Vec<ListedSubject> {
    let event = match event {
        Some(e) => e,
        None => return Vec::new(),
    };
    let fava_event = fava::EventValue::Signed(event.clone());
    SimpleGroupAdmins::from_event(&fava_event)
        .ok()
        .map(|decoded| {
            decoded
                .admins()
                .iter()
                .filter_map(|entry| entry.as_ref().ok())
                .filter_map(|(key_hex, roles)| {
                    let pubkey = nostr::key::PublicKey::from_hex(key_hex).ok()?;
                    Some(ListedSubject::new(pubkey, roles.clone()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn decode_members(event: Option<&Event>) -> Vec<ListedSubject> {
    let event = match event {
        Some(e) => e,
        None => return Vec::new(),
    };
    let fava_event = fava::EventValue::Signed(event.clone());
    SimpleGroupMembers::from_event(&fava_event)
        .ok()
        .map(|decoded| {
            decoded
                .members()
                .iter()
                .filter_map(|entry| entry.as_ref().ok())
                .filter_map(|key_hex| {
                    let pubkey = nostr::key::PublicKey::from_hex(key_hex).ok()?;
                    Some(ListedSubject::new(pubkey, Vec::new()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn d_tag(event: &Event) -> Option<&str> {
    event.tags.iter().find_map(|t| {
        let s = t.as_slice();
        if s.first().map(String::as_str) == Some("d") {
            s.get(1).map(String::as_str)
        } else {
            None
        }
    })
}
