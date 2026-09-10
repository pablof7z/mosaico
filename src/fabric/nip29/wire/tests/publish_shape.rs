//! Composing a group event the way the product actually composes one.

use super::super::*;
use nostr::event::FinalizeEvent;
use nostr::key::Keys;

/// Encode and sign exactly as the publish path does.
///
/// The wire codec composes the body; the NIP-29 provider adds every `h` row;
/// only then is it signed. A test that signs the codec's output directly is testing
/// bytes Mosaico never publishes -- and would not notice the codec silently
/// dropping the context row, because it never had one to drop.
pub(in crate::fabric::nip29::wire) fn signed_as_published(ev: &DomainEvent, keys: &Keys) -> Event {
    let builder = Nip29WireCodec.encode_event(ev).expect("encode");
    match ev {
        // A profile is not a group write at all.
        DomainEvent::Profile(_) => builder.finalize(keys).expect("sign"),
        // The several-group write: the provider adds one `h` row per occupied channel.
        DomainEvent::Status(status) => {
            let groups: Vec<&str> = status.channels.iter().map(String::as_str).collect();
            crate::fabric::nip29::signed_into_groups(&groups, builder, keys).unwrap()
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
    crate::fabric::nip29::signed_into_group(group, builder, keys)
}
