//! Fabric abstraction layer around NMP I/O and product event decoding.
//!
//! Layering intent:
//!   Acquisition + all durable writes ← NMP
//!   Profile indexer + bounded reads  ← NMP
//!   NostrEventCodec (encode, decode)  ← Nip29WireCodec
//!   Process-local presentation views  ← NMP deliveries

pub(crate) mod group_management;
pub mod nip29;
pub mod provider;

/// Raw envelope currently emitted by the Nostr delivery path.
///
/// This is intentionally not advertised as transport-neutral: current decoding
/// is NIP-29-over-Nostr-specific. Future providers should own their native
/// envelope type instead of making this enum a cross-fabric dumping ground.
pub enum RawEnvelope {
    Nostr(nostr::Event),
}

/// Encode/decode between `DomainEvent` and Nostr event envelopes.
///
/// The return type is `nostr::EventBuilder`, so this boundary is explicitly
/// Nostr-specific even when a concrete codec maps NIP-29 group semantics.
pub trait NostrEventCodec {
    fn encode(&self, ev: &crate::domain::DomainEvent) -> anyhow::Result<nostr::EventBuilder>;
    fn decode(&self, env: &RawEnvelope) -> Option<crate::domain::DomainEvent>;
}

// ── Product decode output ─────────────────────────────────────────────────────

#[derive(Default)]
pub struct ProductDecode {
    /// The decoded domain event to forward onto the tail channel, if any.
    /// Emitted for every successfully decoded event, including is_self. For
    /// chat this is the enriched event (sender slug resolved from the store),
    /// so tail consumers never see an empty slug.
    pub tail: Option<crate::domain::DomainEvent>,
}

// ── Top-level dispatcher ──────────────────────────────────────────────────────

/// Decode one NMP-delivered envelope for product side effects.
///
/// Relay-derived state remains in NMP's delivered view. This function may read
/// that view to enrich a display value, but it never persists a second copy.
pub fn decode_product_event(env: &RawEnvelope, store: &crate::state::Store) -> ProductDecode {
    use crate::domain::DomainEvent;
    use crate::fabric::nip29::wire::Nip29WireCodec;

    let RawEnvelope::Nostr(event) = env;

    // Group records are projected by NMP's GroupObservation, not decoded into
    // product events here.
    if matches!(event.kind.as_u16(), 39000..=39002) {
        return ProductDecode::default();
    }

    let codec = Nip29WireCodec;
    let Some(de) = codec.decode(env) else {
        return ProductDecode::default();
    };

    let mut decoded = ProductDecode {
        tail: Some(de.clone()),
    };

    match de {
        DomainEvent::Profile(_) | DomainEvent::Status(_) => {}

        DomainEvent::ChatMessage(ref chat) => {
            let sender_pk = event.pubkey.to_hex();
            if let Some(slug) = store
                .resolve_slug_for_pubkey(&sender_pk)
                .ok()
                .flatten()
                .filter(|slug| !slug.is_empty())
            {
                decoded.tail = Some(DomainEvent::ChatMessage(crate::domain::ChatMessage {
                    from: crate::domain::AgentRef::new(sender_pk, slug),
                    ..chat.clone()
                }));
            }
        }

        // Reactions (kind:7) are passive awareness read from the NMP reaction
        // projection, so they never enter direct-mention routing,
        // wake an idle agent, or inject mid-turn. No tail (nothing live-delivers).
        DomainEvent::Reaction(_) => {
            decoded.tail = None;
        }
    }

    decoded
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{RegisterSession, Store, TestGroup, TestGroupDelivery};
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn make_tag(parts: &[&str]) -> Tag {
        Tag::parse(parts.iter().copied()).unwrap()
    }

    fn build_event(keys: &Keys, kind_n: u16, content: &str, tags: Vec<Tag>) -> nostr::Event {
        EventBuilder::new(Kind::from(kind_n), content)
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }

    fn relay_event(event: &nostr::Event) -> crate::state::RelayEvent {
        crate::state::RelayEvent {
            id: event.id.to_hex(),
            kind: event.kind.as_u16() as u32,
            pubkey: event.pubkey.to_hex(),
            created_at: event.created_at.as_secs(),
            channel_h: crate::fabric::nip29::nostr_tag(event, "h")
                .unwrap_or_default()
                .into(),
            d_tag: crate::fabric::nip29::nostr_tag(event, "d")
                .unwrap_or_default()
                .into(),
            content: event.content.clone(),
            tags_json: serde_json::to_string(
                &event
                    .tags
                    .iter()
                    .map(|tag| tag.as_slice().to_vec())
                    .collect::<Vec<_>>(),
            )
            .unwrap(),
        }
    }

    fn register(store: &Store, pubkey: &str, channel_h: &str, agent_slug: &str) {
        store
            .reserve_hook_session_for_test(&RegisterSession {
                pubkey: pubkey.into(),
                observed_harness: "claude-code".into(),
                agent_slug: agent_slug.into(),
                launch_channel_h: channel_h.into(),
                work_root: channel_h.into(),
                child_pid: None,
                now: 1,
            })
            .unwrap();
    }

    #[test]
    fn chat_decode_reads_observed_recipients_without_executing_delivery() {
        let store = Store::open_memory().unwrap();
        let sender_keys = Keys::generate();
        let receiver_keys = Keys::generate();
        let sender_pk = sender_keys.public_key().to_hex();
        let receiver_pk = receiver_keys.public_key().to_hex();

        register(&store, &sender_pk, "mychannel", "sender-ext");
        register(&store, &receiver_pk, "mychannel", "receiver-ext");
        store.install_test_nmp_group_delivery(TestGroupDelivery::new([TestGroup::new(
            "mychannel",
        )
        .admins(Vec::new())
        .members(vec![sender_pk.clone(), receiver_pk.clone()])]));

        // Ambient message (no p-tag): observed by NMP, inbox stays empty.
        let ambient = build_event(
            &sender_keys,
            9,
            "heads up: I pushed the parser fix",
            vec![make_tag(&["h", "mychannel"])],
        );
        store.install_test_nmp_relay_delivery(
            crate::state::TestRelayDelivery::new().events([relay_event(&ambient)]),
        );
        decode_product_event(&RawEnvelope::Nostr(ambient.clone()), &store);
        assert!(store
            .peek_pending_for_pubkey(&receiver_pk)
            .unwrap()
            .is_empty());
        assert!(store.has_event(&ambient.id.to_hex()).unwrap());

        // Mention execution belongs to the daemon ownership router. The
        // transport projection preserves the explicit recipient for reads and
        // search without parking an executable inbox row.
        let mention = build_event(
            &sender_keys,
            9,
            "hey receiver, LGTM",
            vec![
                make_tag(&["h", "mychannel"]),
                make_tag(&["p", &receiver_pk]),
            ],
        );
        store.install_test_nmp_relay_delivery(
            crate::state::TestRelayDelivery::new()
                .events([relay_event(&ambient), relay_event(&mention)]),
        );
        decode_product_event(&RawEnvelope::Nostr(mention.clone()), &store);
        let receiver_rows = store.peek_pending_for_pubkey(&receiver_pk).unwrap();
        assert!(receiver_rows.is_empty());
        assert_eq!(
            store.message_recipients(&mention.id.to_hex()).unwrap()[0].recipient_pubkey,
            receiver_pk
        );
        assert!(
            store
                .peek_pending_for_pubkey(&sender_pk)
                .unwrap()
                .is_empty(),
            "sender session should not receive its own chat line"
        );
    }

    #[test]
    fn reaction_decodes_to_passive_projection_only_and_never_wakes() {
        let store = Store::open_memory().unwrap();
        let author_keys = Keys::generate();
        let reactor_keys = Keys::generate();
        let author_pk = author_keys.public_key().to_hex();

        // Seed a message authored by `author` so the reaction join resolves.
        let chat = build_event(
            &author_keys,
            9,
            "pushed the fix",
            vec![make_tag(&["h", "c"])],
        );
        let target_id = chat.id.to_hex();
        let reaction = build_event(
            &reactor_keys,
            7,
            "👍",
            vec![make_tag(&["e", &target_id]), make_tag(&["h", "c"])],
        );
        store.install_test_nmp_relay_delivery(
            crate::state::TestRelayDelivery::new()
                .events([relay_event(&chat), relay_event(&reaction)]),
        );
        let outcome = decode_product_event(&RawEnvelope::Nostr(reaction.clone()), &store);

        // Passive: no tail, no wake, no inbox row, no recipient edge.
        assert!(outcome.tail.is_none(), "reaction emits no tail");
        assert!(
            store.message_recipients(&target_id).unwrap().is_empty(),
            "reaction writes no recipient edge (no inject path)"
        );

        // Exactly one reaction row, joined to the target body.
        let rows = store
            .reactions_on_authored_after(&author_pk, 0, 10)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].emoji, "👍");
        assert_eq!(rows[0].target_body, "pushed the fix");

        // Decoding the same observed row again does not create product state.
        decode_product_event(&RawEnvelope::Nostr(reaction), &store);
        let rows = store
            .reactions_on_authored_after(&author_pk, 0, 10)
            .unwrap();
        assert_eq!(rows.len(), 1, "replayed reaction stays a single row");
    }
}
