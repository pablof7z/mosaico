//! Turning a Mosaico draft into the shape NMP's NIP-29 group door takes.
//!
//! Nothing here knows what an `h` row is. `nmp::nip29::Group` owns the group
//! context tag on both mint doors -- appended before signing on
//! [`Group::intent`](nmp::nip29::Group::intent), validated rather than
//! repaired on [`Group::signed_intent`](nmp::nip29::Group::signed_intent) --
//! and refuses a draft that carries one already. What is left for this module
//! is the two mechanical conversions NMP cannot do for us: `nostr`'s builder
//! into `nmp`'s, and a minted intent's frozen body into the id it will have.

use anyhow::{Context, Result};
use nmp_grammar::{WriteIntent, WritePayload};
use nostr::{EventId, PublicKey, UnsignedEvent};

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

/// The id a minted intent will carry, whichever half of the lifecycle it is in.
///
/// A signed payload already has one. A builder payload has the frozen body NMP
/// will sign, and its `h` row is inside those bytes because the group door put
/// it there before this was read back.
pub(super) fn frozen_id_of(intent: &WriteIntent, author: PublicKey) -> Result<EventId> {
    match &intent.payload {
        WritePayload::Event(builder) => frozen_event_id(builder, author),
        WritePayload::Signed(event) => Ok(event.id),
        // The group door mints exactly the two payloads above, from
        // `Group::intent` and `Group::signed_intent`. A third one arriving
        // here means the door grew a shape Mosaico has not read.
        _ => anyhow::bail!(
            "NMP's group door minted a payload shape Mosaico has not read; \
             `frozen_id_of` must learn it before this write can be tracked"
        ),
    }
}

/// One Mosaico draft as the group door's builder.
///
/// `created_at` is stated rather than left absent: Mosaico's own doors hand
/// back the frozen id synchronously, and an id cannot be computed for a
/// timestamp NMP has not stamped yet. NMP keeps a stated one verbatim.
///
/// The tags cross unchanged, including any `h` the caller wrongly supplied --
/// the group door refuses that with `CallerSuppliedContext`, and letting the
/// refusal happen there is what keeps the rule in one place.
pub(super) fn draft_of(unsigned: UnsignedEvent) -> nmp::EventBuilder {
    nmp::EventBuilder {
        kind: unsigned.kind,
        tags: unsigned.tags.into_iter().collect(),
        content: unsigned.content,
        created_at: Some(unsigned.created_at),
    }
}

/// A group-contextualized builder back as an unsigned event, so Mosaico can
/// sign the bytes the group door composed rather than bytes of its own.
pub(super) fn unsigned_of(builder: nmp::EventBuilder, author: PublicKey) -> UnsignedEvent {
    UnsignedEvent {
        id: None,
        pubkey: author,
        created_at: builder.created_at.unwrap_or_else(nostr::Timestamp::now),
        kind: builder.kind,
        tags: nostr::Tags::from_list(builder.tags),
        content: builder.content,
    }
}
