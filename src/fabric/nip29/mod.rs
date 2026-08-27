//! NIP-29 fabric adapter — group lifecycle, observation, and product codecs.

pub mod lifecycle;
pub mod orchestration;
pub mod readiness;
pub mod session_dispatch;
pub mod wire;

/// Compose the exact bytes NMP's group door would sign, for a fixture.
///
/// Test-only, and it calls NMP's own contextualizer rather than restating the
/// rule: a fixture that wrote its own `h` row would be modelling a shape the
/// product cannot publish, because `nip29::Groups::publish` refuses a
/// caller-supplied context row. Product code never composes a group event --
/// it hands NMP a draft and gets back an id.
#[cfg(test)]
pub(crate) fn signed_into_groups(
    groups: &[&str],
    builder: nostr::event::EventBuilder,
    keys: &nostr::key::Keys,
) -> nostr::event::Event {
    use nostr::event::FinalizeEvent;
    let h_tags: Vec<nostr::event::Tag> = groups
        .iter()
        .map(|g| nostr::event::Tag::parse(["h", g]).expect("h tag"))
        .collect();
    builder
        .tags(h_tags)
        .finalize(keys)
        .expect("signing a fixture")
}

/// [`signed_into_groups`] for the ordinary one-group case.
#[cfg(test)]
pub(crate) fn signed_into_group(
    group: &str,
    builder: nostr::event::EventBuilder,
    keys: &nostr::key::Keys,
) -> nostr::event::Event {
    signed_into_groups(&[group], builder, keys)
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
