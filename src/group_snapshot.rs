//! Mosaico's local projection of relay-authoritative group records.
//!
//! These product-facing values are decoded from a complete current Nostr level.

use std::collections::BTreeSet;

use nostr::event::Event;
use nostr::key::PublicKey;

// ---------------------------------------------------------------- availability

/// Whether relay-authoritative group records are currently available.
///
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupAvailability {
    /// Relay-live records arrived; the snapshot reflects current relay state.
    Ready,
}

// ------------------------------------------------------------------ subjects

/// One relay-signed admin or member entry.
///
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
        Self {
            name,
            about,
            tags,
            as_of,
        }
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
/// The `id` is the `d` tag value.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupSnapshot {
    pub id: String,
    pub metadata: Option<GroupMetadata>,
    pub admins: Vec<ListedSubject>,
    pub members: Vec<ListedSubject>,
    pub availability: GroupAvailability,
}

// ------------------------------------------------------------ decode helpers

/// Decode one complete relay-authored group-record level into product values.
///
/// `group_ids` restricts which `d` values are included (empty = all).
/// `default_av` is used when relay EOSE is not yet complete.
pub(crate) fn decode_snapshots(
    events: &[Event],
    group_ids: &BTreeSet<String>,
    default_av: GroupAvailability,
) -> Vec<GroupSnapshot> {
    use std::collections::BTreeMap;

    let mut metadata_map: BTreeMap<String, Event> = BTreeMap::new();
    let mut admins_map: BTreeMap<String, Event> = BTreeMap::new();
    let mut members_map: BTreeMap<String, Event> = BTreeMap::new();

    for event in events {
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

    let av = default_av;

    all_ids
        .into_iter()
        .map(|id| {
            let metadata = metadata_map
                .get(&id)
                .map(GroupMetadata::from_metadata_event);
            let admins = decode_admins(admins_map.get(&id));
            let members = decode_members(members_map.get(&id));
            GroupSnapshot {
                id,
                metadata,
                admins,
                members,
                availability: av,
            }
        })
        .collect()
}

fn insert_latest(map: &mut std::collections::BTreeMap<String, Event>, key: String, event: Event) {
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
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let tag = tag.as_slice();
            (tag.first().map(String::as_str) == Some("p"))
                .then(|| {
                    nostr::key::PublicKey::from_hex(tag.get(1)?)
                        .ok()
                        .map(|pubkey| {
                            ListedSubject::new(pubkey, tag.iter().skip(2).cloned().collect())
                        })
                })
                .flatten()
        })
        .collect()
}

fn decode_members(event: Option<&Event>) -> Vec<ListedSubject> {
    let event = match event {
        Some(e) => e,
        None => return Vec::new(),
    };
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let tag = tag.as_slice();
            (tag.first().map(String::as_str) == Some("p"))
                .then(|| {
                    nostr::key::PublicKey::from_hex(tag.get(1)?)
                        .ok()
                        .map(|pubkey| ListedSubject::new(pubkey, Vec::new()))
                })
                .flatten()
        })
        .collect()
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
