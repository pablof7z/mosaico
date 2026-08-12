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
    builder: nostr::EventBuilder,
    keys: &nostr::Keys,
) -> nostr::Event {
    let unsigned = builder.build(keys.public_key());
    let contextualized = nmp_nip29::contextualize(
        &groups.iter().map(|g| (*g).to_string()).collect(),
        nmp::EventBuilder {
            kind: unsigned.kind,
            tags: unsigned.tags.into_iter().collect(),
            content: unsigned.content,
            created_at: Some(unsigned.created_at),
        },
    )
    .expect("a fixture draft carries no context row of its own");
    nostr::EventBuilder::new(contextualized.kind, contextualized.content)
        .tags(contextualized.tags)
        .custom_created_at(contextualized.created_at.expect("stated above"))
        .allow_self_tagging()
        .sign_with_keys(keys)
        .expect("signing a fixture")
}

/// [`signed_into_groups`] for the ordinary one-group case.
#[cfg(test)]
pub(crate) fn signed_into_group(
    group: &str,
    builder: nostr::EventBuilder,
    keys: &nostr::Keys,
) -> nostr::Event {
    signed_into_groups(&[group], builder, keys)
}

/// Read a single tag value by name from a Nostr event.
///
/// This is a small helper local to the fabric crate; it does NOT disturb the
/// `event_tag` helper in `daemon/server.rs`.
pub(crate) fn nostr_tag<'a>(event: &'a nostr::Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|t| {
        let s = t.as_slice();
        if s.first().map(String::as_str) == Some(name) {
            s.get(1).map(String::as_str)
        } else {
            None
        }
    })
}
