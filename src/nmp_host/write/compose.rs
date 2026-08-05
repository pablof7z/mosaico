//! Composing a NIP-29 group write body.
//!
//! One body is composed per write and shared by every host's intent: all hosts
//! receive identical bytes, so one frozen event id names the write everywhere.

use anyhow::{Context, Result};
use nmp::RelayUrl;
use nmp_grammar::{Identity, WriteIntent, WritePayload, WriteRouting};
use nostr::{Event, EventId, PublicKey, Tag, UnsignedEvent};
use std::collections::BTreeSet;

/// The id NMP freezes at acceptance for a builder payload, by NIP-01's own
/// rule. NMP applies exactly this (`freeze_payload`) to the same fields, and a
/// stated `created_at` is kept verbatim, so the two agree by construction.
pub(super) fn frozen_event_id(builder: &nmp::EventBuilder, author: PublicKey) -> Result<EventId> {
    let created_at = builder
        .created_at
        .context("a NIP-29 group write must state its created_at before acceptance")?;
    Ok(EventId::new(
        &author,
        &created_at,
        &builder.kind,
        &nostr::Tags::from_list(builder.tags.clone()),
        &builder.content,
    ))
}

#[derive(Clone)]
pub(super) struct GroupTemplate {
    pub(super) group: String,
    pub(super) created_at: nostr::Timestamp,
    pub(super) kind: u16,
    pub(super) content: String,
    pub(super) extra_tags: Vec<Vec<String>>,
}

pub(super) fn unsigned_template(unsigned: &UnsignedEvent) -> Result<GroupTemplate> {
    group_template(
        unsigned.created_at,
        unsigned.kind.as_u16(),
        unsigned.content.clone(),
        unsigned.tags.iter().collect(),
    )
}

pub(super) fn event_template(event: &Event) -> Result<GroupTemplate> {
    group_template(
        event.created_at,
        event.kind.as_u16(),
        event.content.clone(),
        event.tags.iter().collect(),
    )
}

pub(super) fn group_template(
    created_at: nostr::Timestamp,
    kind: u16,
    content: String,
    tags: Vec<&Tag>,
) -> Result<GroupTemplate> {
    let groups = group_values(tags.iter().copied());
    let group = groups
        .first()
        .cloned()
        .context("NIP-29 write has no h tag")?;
    let extra_tags = tags
        .into_iter()
        .filter(|tag| {
            !matches!(
                tag.as_slice().first().map(String::as_str),
                Some("h" | "previous")
            )
        })
        .map(|tag| tag.as_slice().to_vec())
        .collect();
    Ok(GroupTemplate {
        group,
        created_at,
        kind,
        content,
        extra_tags,
    })
}

pub(super) fn group_values<'a>(tags: impl IntoIterator<Item = &'a Tag>) -> BTreeSet<String> {
    tags.into_iter()
        .filter_map(|tag| {
            let row = tag.as_slice();
            (row.first().map(String::as_str) == Some("h"))
                .then(|| row.get(1).cloned())
                .flatten()
        })
        .collect()
}

/// Mint the group write body from NMP's `#h` contextualizer.
/// `nmp_nip29::contextualize` refuses a caller-supplied `h` or `previous` row,
/// so `group_template` strips both before we get here.
///
/// The body is composed once and shared by every host's intent: all hosts
/// receive identical bytes, so they share one frozen id.
///
/// Routing and identity are assembled by the caller only because
/// `nmp::nip29::Group` exposes no mint-without-publish door: `Group::intent`
/// and `through_the_one_door` are private, and `publish`/`publish_signed`
/// publish immediately and return receipts. Mosaico mints intents and submits
/// them later through one choke-point, so it cannot use a publish-now door.
/// See NMP #1242; this is a visible gap, not a deliberate bypass of the group.
pub(super) fn contextualized_builder(template: GroupTemplate) -> Result<nmp::EventBuilder> {
    let tags = template
        .extra_tags
        .into_iter()
        .map(|row| {
            Tag::parse(row).map_err(|error| anyhow::anyhow!("invalid NIP-29 tag: {error:?}"))
        })
        .collect::<Result<Vec<_>>>()?;
    let builder = nmp::EventBuilder {
        kind: nostr::Kind::from(template.kind),
        tags,
        content: template.content,
        created_at: Some(template.created_at),
    };
    nmp_nip29::contextualize(&template.group, builder)
        .map_err(|error| anyhow::anyhow!("composing NMP group write: {error:?}"))
}

/// One host's intent for an already-composed group body.
pub(super) fn group_intent(relay: RelayUrl, builder: nmp::EventBuilder) -> WriteIntent {
    WriteIntent {
        payload: WritePayload::Event(builder),
        routing: WriteRouting::Explicit(vec![relay]),
        // A group write says nothing about WHO is publishing; callers that
        // know the author overwrite this with `Identity::Explicit`.
        identity: Identity::Active,
        correlation: None,
    }
}
