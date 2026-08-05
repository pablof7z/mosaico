//! Composing a group event the way the product actually composes one.

use super::super::*;

/// Encode and sign exactly as the publish path does.
///
/// The wire codec composes the body; NMP's group door mints the `h` row; only
/// then is it signed. A test that signs the codec's output directly is testing
/// bytes Mosaico never publishes -- and would not notice the codec silently
/// dropping the context row, because it never had one to drop.
pub(in crate::fabric::nip29::wire) fn signed_as_published(ev: &DomainEvent, keys: &Keys) -> Event {
    let builder = Nip29WireCodec.encode_event(ev).expect("encode");
    match ev {
        // Already carries its own rows -- the multi-group write no door mints.
        DomainEvent::Status(_) | DomainEvent::Profile(_) => {
            builder.sign_with_keys(keys).expect("sign")
        }
        DomainEvent::ChatMessage(chat) => sign_into(&chat.channel, builder, keys),
        DomainEvent::Reaction(reaction) => sign_into(&reaction.channel, builder, keys),
    }
}

pub(in crate::fabric::nip29::wire) fn sign_into(
    group: &str,
    builder: EventBuilder,
    keys: &Keys,
) -> Event {
    crate::nmp_host::write::contextualized_draft(group, builder, keys.public_key())
        .expect("a draft with no context row of its own")
        .sign_with_keys(keys)
        .expect("sign")
}
