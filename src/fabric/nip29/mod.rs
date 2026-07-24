//! NIP-29 fabric adapter — group metadata and membership materializer.

pub mod lifecycle;
pub mod materializer;
pub mod orchestration;
pub mod readiness;
pub mod session_dispatch;
pub mod wire;

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
