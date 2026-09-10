//! NIP-29 fabric adapter — group lifecycle, observation, and product codecs.

use anyhow::Context;

pub mod lifecycle;
pub(crate) mod management;
pub mod orchestration;
pub mod readiness;
pub mod session_dispatch;
pub(crate) mod view;
pub mod wire;

/// Complete a product draft with every NIP-29 group context before signing.
pub(crate) fn signed_into_groups(
    groups: &[&str],
    builder: nostr::event::EventBuilder,
    keys: &nostr::key::Keys,
) -> anyhow::Result<nostr::event::Event> {
    use nostr::event::FinalizeEvent;
    let h_tags: Vec<nostr::event::Tag> = groups
        .iter()
        .map(|group| {
            if group.is_empty() {
                anyhow::bail!("a NIP-29 publication requires a non-empty group id");
            }
            nostr::event::Tag::parse(["h", group])
                .map_err(anyhow::Error::from)
                .context("building NIP-29 group tag")
        })
        .collect::<anyhow::Result<_>>()?;
    builder
        .tags(h_tags)
        .finalize(keys)
        .context("signing NIP-29 event")
}

/// [`signed_into_groups`] for the ordinary one-group case.
#[cfg(test)]
pub(crate) fn signed_into_group(
    group: &str,
    builder: nostr::event::EventBuilder,
    keys: &nostr::key::Keys,
) -> nostr::event::Event {
    signed_into_groups(&[group], builder, keys).expect("signing a fixture")
}

/// Read a single tag value by name from a Nostr event.
///
/// This is a small helper local to the fabric crate; it does NOT disturb the
/// `event_tag` helper in `daemon/server.rs`.
pub(crate) fn nostr_tag<'a>(event: &'a nostr::event::Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|t| {
        let s = t.as_slice();
        if s.first().map(String::as_str) == Some(name) {
            s.get(1).map(String::as_str)
        } else {
            None
        }
    })
}
