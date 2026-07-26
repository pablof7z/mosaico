//! Fabric abstraction layer around NMP I/O and provider materialization.
//!
//! Layering intent:
//!   Acquisition + all durable writes ← NMP
//!   Profile indexer + bounded reads  ← NMP
//!   NostrEventCodec (encode, decode)  ← Nip29WireCodec
//!   Materializer (store writes)       ← materialize()

pub(crate) mod group_management;
pub mod nip29;
pub mod provider;

/// Raw envelope currently emitted by the Nostr delivery path.
///
/// This is intentionally not advertised as transport-neutral: current materialization
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

// ── Materializer output ───────────────────────────────────────────────────────

#[derive(Default)]
pub struct MaterializationOutcome {
    /// The decoded domain event to forward onto the tail channel, if any.
    /// Emitted for every successfully decoded event, including is_self. For
    /// chat this is the enriched event (sender slug resolved from the store),
    /// so tail consumers never see an empty slug.
    pub tail: Option<crate::domain::DomainEvent>,
}

// ── Top-level dispatcher ──────────────────────────────────────────────────────

/// Decode one raw envelope and apply all store side-effects.
///
/// Every observed event is materialized into one cache by kind.
/// Relay acceptance is the sender/channel admission boundary. Chat is cached
/// without a second local membership decision; daemon-owned p-tag execution is
/// handled by the server after materialization.
pub fn materialize(env: &RawEnvelope, store: &crate::state::Store) -> MaterializationOutcome {
    use crate::domain::DomainEvent;
    use crate::fabric::nip29::materializer::Nip29Materializer;
    use crate::fabric::nip29::wire::Nip29WireCodec;

    let RawEnvelope::Nostr(event) = env;

    // Relay-authored NIP-29 state events go straight to their dedicated caches and
    // never decode into a domain event (no tail).
    match event.kind.as_u16() {
        39000 => {
            Nip29Materializer::materialize_channel(store, event);
            return MaterializationOutcome::default();
        }
        39001 => {
            Nip29Materializer::materialize_admins(store, event);
            return MaterializationOutcome::default();
        }
        39002 => {
            Nip29Materializer::materialize_members(store, event);
            return MaterializationOutcome::default();
        }
        _ => {}
    }

    // Unknown kinds land in relay_events except dedicated-cache kinds.
    let codec = Nip29WireCodec;
    let Some(de) = codec.decode(env) else {
        let k = event.kind.as_u16();
        if k != 0 && k != 30315 {
            Nip29Materializer::materialize_event(store, event);
        }
        return MaterializationOutcome::default();
    };

    let created_at = event.created_at.as_secs();
    let mut outcome = MaterializationOutcome {
        tail: Some(de.clone()),
    };

    match de {
        DomainEvent::Profile(ref pf) => {
            Nip29Materializer::materialize_profile(store, pf, created_at);
        }

        DomainEvent::Status(ref st) => {
            Nip29Materializer::materialize_status(store, st, created_at);
        }

        DomainEvent::ChatMessage(ref chat) => {
            Nip29Materializer::materialize_event(store, event);
            Nip29Materializer::materialize_chat_message(store, event, chat);
            let sender_pk = event.pubkey.to_hex();
            if let Some(slug) = store
                .resolve_slug_for_pubkey(&sender_pk)
                .ok()
                .flatten()
                .filter(|slug| !slug.is_empty())
            {
                outcome.tail = Some(DomainEvent::ChatMessage(crate::domain::ChatMessage {
                    from: crate::domain::AgentRef::new(sender_pk, slug),
                    ..chat.clone()
                }));
            }
        }

        // Reactions (kind:7) are passive awareness: written to the reactions
        // projection ONLY, so a reaction can never enter direct-mention routing,
        // wake an idle agent, or inject mid-turn. No tail (nothing live-delivers).
        DomainEvent::Reaction(ref rx) => {
            Nip29Materializer::materialize_reaction(store, event, rx);
            outcome.tail = None;
        }
    }

    outcome
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{RegisterSession, Store};
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
    fn chat_materialization_stays_transport_only() {
        let store = Store::open_memory().unwrap();
        let sender_keys = Keys::generate();
        let receiver_keys = Keys::generate();
        let sender_pk = sender_keys.public_key().to_hex();
        let receiver_pk = receiver_keys.public_key().to_hex();

        register(&store, &sender_pk, "mychannel", "sender-ext");
        register(&store, &receiver_pk, "mychannel", "receiver-ext");
        store.replace_channel_admins("mychannel", &[], 1).unwrap();
        store
            .replace_channel_members("mychannel", &[sender_pk.clone(), receiver_pk.clone()], 1)
            .unwrap();

        // Ambient message (no p-tag): stored in relay_events, inbox stays empty.
        let ambient = build_event(
            &sender_keys,
            9,
            "heads up: I pushed the parser fix",
            vec![make_tag(&["h", "mychannel"])],
        );
        materialize(&RawEnvelope::Nostr(ambient.clone()), &store);
        assert!(store
            .peek_pending_for_pubkey(&receiver_pk)
            .unwrap()
            .is_empty());
        assert!(store.has_event(&ambient.id.to_hex()).unwrap());

        // Mention execution belongs to the daemon ownership router, not this
        // transport projection.
        let mention = build_event(
            &sender_keys,
            9,
            "hey receiver, LGTM",
            vec![
                make_tag(&["h", "mychannel"]),
                make_tag(&["p", &receiver_pk]),
            ],
        );
        materialize(&RawEnvelope::Nostr(mention.clone()), &store);
        let receiver_rows = store.peek_pending_for_pubkey(&receiver_pk).unwrap();
        assert!(receiver_rows.is_empty());
        assert!(store
            .message_recipients(&mention.id.to_hex())
            .unwrap()
            .is_empty());
        assert!(
            store
                .peek_pending_for_pubkey(&sender_pk)
                .unwrap()
                .is_empty(),
            "sender session should not receive its own chat line"
        );
    }

    #[test]
    fn group_metadata_materializes_into_relay_channels() {
        let store = Store::open_memory().unwrap();
        let relay = Keys::generate();
        let event = build_event(
            &relay,
            39000,
            "",
            vec![make_tag(&["d", "proj"]), make_tag(&["name", "Channel"])],
        );
        let env = RawEnvelope::Nostr(event);
        let outcome = materialize(&env, &store);
        assert!(outcome.tail.is_none(), "relay-authored state has no tail");
        assert_eq!(store.get_channel("proj").unwrap().unwrap().name, "Channel");
    }

    #[test]
    fn reaction_materializes_to_projection_only_and_never_wakes() {
        use crate::state::RecordMessage;
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
        store
            .record_message(&RecordMessage {
                message_id: target_id.clone(),
                thread_id: "c".into(),
                channel_h: "c".into(),
                author_pubkey: author_pk.clone(),
                body: "pushed the fix".into(),
                created_at: 100,
                direction: "outbound".into(),
                sync_state: "accepted".into(),
                native_event_id: Some(target_id.clone()),
                error: None,
            })
            .unwrap();

        let reaction = build_event(
            &reactor_keys,
            7,
            "👍",
            vec![make_tag(&["e", &target_id]), make_tag(&["h", "c"])],
        );
        let outcome = materialize(&RawEnvelope::Nostr(reaction.clone()), &store);

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

        // Replaying the same event is idempotent.
        materialize(&RawEnvelope::Nostr(reaction), &store);
        let rows = store
            .reactions_on_authored_after(&author_pk, 0, 10)
            .unwrap();
        assert_eq!(rows.len(), 1, "replayed reaction stays a single row");
    }
    #[test]
    fn unknown_kind_is_cached_verbatim() {
        let store = Store::open_memory().unwrap();
        let agent = Keys::generate();
        // kind:7 (reaction) is not decoded by the codec but must still be cached.
        let event = build_event(&agent, 7, "+", vec![make_tag(&["h", "proj"])]);
        let env = RawEnvelope::Nostr(event.clone());
        materialize(&env, &store);
        assert!(store.has_event(&event.id.to_hex()).unwrap());
    }
}
