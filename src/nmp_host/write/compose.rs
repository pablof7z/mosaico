//! The one mechanical conversion NMP's group door cannot do for us.
//!
//! Nothing here knows what an `h` row is, what a NIP-29 kind means, or what an
//! event id is made of. `nmp::nip29::Groups` owns the context rows, the
//! stamping, the signature and the id; Mosaico owns the draft. All that is
//! left in between is `nostr`'s builder shape becoming `nmp`'s.

use nostr::UnsignedEvent;

/// One Mosaico draft as the group door's builder.
///
/// `created_at` crosses verbatim because it is the app's own statement about
/// when it composed the message, and NMP keeps a stated one rather than
/// re-stamping it. Nothing downstream derives an id from it any more.
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
