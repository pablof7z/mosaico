use super::Nip29Provider;
use crate::fabric::nip29::{nostr_tag, wire};
use crate::fabric::{MaterializationOutcome, RawEnvelope};
use crate::state::Store;

impl Nip29Provider {
    /// Decode one raw envelope and apply all store side-effects.
    pub fn materialize(&self, env: &RawEnvelope, store: &Store) -> MaterializationOutcome {
        let outcome = crate::fabric::materialize(env, store);
        if let Some(channel) = roster_snapshot_channel(env) {
            self.readiness.invalidate_channel(channel);
        }
        outcome
    }
}

fn roster_snapshot_channel(env: &RawEnvelope) -> Option<&str> {
    let RawEnvelope::Nostr(event) = env;
    match event.kind.as_u16() {
        wire::KIND_GROUP_ADMINS | wire::KIND_GROUP_MEMBERS => nostr_tag(event, "d"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn event(kind: u16, tags: Vec<Tag>) -> RawEnvelope {
        RawEnvelope::Nostr(
            EventBuilder::new(Kind::from(kind), "")
                .tags(tags)
                .sign_with_keys(&Keys::generate())
                .unwrap(),
        )
    }

    fn tag(parts: &[&str]) -> Tag {
        Tag::parse(parts.iter().copied()).unwrap()
    }

    #[test]
    fn roster_snapshots_identify_readiness_invalidation_channel() {
        let admins = event(wire::KIND_GROUP_ADMINS, vec![tag(&["d", "chan"])]);
        let members = event(wire::KIND_GROUP_MEMBERS, vec![tag(&["d", "chan"])]);
        let metadata = event(wire::KIND_GROUP_METADATA, vec![tag(&["d", "chan"])]);
        let chat = event(wire::KIND_CHAT, vec![tag(&["h", "chan"])]);

        assert_eq!(roster_snapshot_channel(&admins), Some("chan"));
        assert_eq!(roster_snapshot_channel(&members), Some("chan"));
        assert_eq!(roster_snapshot_channel(&metadata), None);
        assert_eq!(roster_snapshot_channel(&chat), None);
    }
}
