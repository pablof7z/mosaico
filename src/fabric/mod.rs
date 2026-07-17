//! Fabric abstraction layer around NMP I/O and provider materialization.
//!
//! Layering intent:
//!   Acquisition + all durable writes ← NMP
//!   NostrEventCodec (encode, decode)  ← Nip29WireCodec
//!   Materializer (store writes)       ← materialize()
//!   Profile indexer + one-shot reads   ← Transport

mod admission;
pub(crate) mod group_management;
pub mod nip29;
pub mod provider;

/// Raw envelope currently emitted by the Nostr delivery path.
///
/// This is intentionally not advertised as transport-neutral: current materialization
/// is NIP-29-over-Nostr-specific. Future providers should own their native
/// envelope type instead of making this enum a cross-fabric dumping ground.
pub enum RawEnvelope {
    Nostr(nostr_sdk::Event),
}

/// Encode/decode between `DomainEvent` and Nostr event envelopes.
///
/// The return type is `nostr_sdk::EventBuilder`, so this boundary is explicitly
/// Nostr-specific even when a concrete codec maps NIP-29 group semantics.
pub trait NostrEventCodec {
    fn encode(&self, ev: &crate::domain::DomainEvent) -> anyhow::Result<nostr_sdk::EventBuilder>;
    fn decode(&self, env: &RawEnvelope) -> Option<crate::domain::DomainEvent>;
}

// ── Materializer output ───────────────────────────────────────────────────────

/// The two side-effects that `handle_incoming` performs outside the store.
/// Admission/quarantine writes happen inside materialization; sync-state
/// reconciliation remains reserved for later phases.
#[derive(Default)]
pub struct MaterializationOutcome {
    /// The decoded domain event to forward onto the tail channel, if any.
    /// Emitted for every successfully decoded event, including is_self. For
    /// routed mentions this is the ENRICHED event (sender slug resolved from
    /// the store), so tail consumers never see an empty slug.
    pub tail: Option<crate::domain::DomainEvent>,
    /// True when a mention was routed and live delivery surfaces should be notified.
    pub wake_mentions: bool,
}

// ── Top-level dispatcher ──────────────────────────────────────────────────────

/// Decode one raw envelope and apply all store side-effects.
///
/// Every observed event is materialized into one cache by kind.
/// Chat (kind:9) is additionally routed into the inbox ledger for local sessions
/// in its channel. The same ledger stores per-target orchestration claims.
///
/// Tail is emitted for every decoded domain event. `wake_mentions` is set only
/// when a chat message is newly routed to a live local session.
///
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
            return MaterializationOutcome {
                tail: None,
                wake_mentions: admission::replay_quarantined_chat(
                    store,
                    crate::fabric::nip29::nostr_tag(event, "d").unwrap_or(""),
                ),
            };
        }
        39002 => {
            Nip29Materializer::materialize_members(store, event);
            return MaterializationOutcome {
                tail: None,
                wake_mentions: admission::replay_quarantined_chat(
                    store,
                    crate::fabric::nip29::nostr_tag(event, "d").unwrap_or(""),
                ),
            };
        }
        crate::fabric::nip29::wire::KIND_AGENT_ROSTER => {
            Nip29Materializer::materialize_agent_roster(store, event);
            return MaterializationOutcome::default();
        }
        _ => {}
    }

    // Unknown kinds land in relay_events except dedicated-cache kinds.
    let codec = Nip29WireCodec;
    let Some(de) = codec.decode(env) else {
        let k = event.kind.as_u16();
        if k != 0 && k != 30315 && k != crate::fabric::nip29::wire::KIND_AGENT_ROSTER {
            Nip29Materializer::materialize_event(store, event);
        }
        return MaterializationOutcome::default();
    };

    let created_at = event.created_at.as_secs();
    let mut outcome = MaterializationOutcome {
        tail: Some(de.clone()),
        wake_mentions: false,
    };

    match de {
        DomainEvent::Profile(ref pf) => {
            Nip29Materializer::materialize_profile(store, pf, created_at);
        }

        DomainEvent::Status(ref st) => {
            Nip29Materializer::materialize_status(store, st, created_at);
        }

        DomainEvent::ChatMessage(ref chat) => {
            outcome = admission::materialize_chat(store, event, chat);
        }

        // Reactions (kind:7) are passive awareness: written to the reactions
        // projection ONLY. `wake_mentions` stays false and this arm never enters
        // `admission::materialize_chat`, so a reaction can never ring a doorbell,
        // wake an idle agent, or inject mid-turn. No tail (nothing live-delivers).
        DomainEvent::Reaction(ref rx) => {
            Nip29Materializer::materialize_reaction(store, event, rx);
            outcome.tail = None;
        }

        // Activity (kind:1) carries no inbox routing; it is cached verbatim in
        // relay_events alongside every other unprojected kind.
        DomainEvent::Activity(_) => {
            Nip29Materializer::materialize_event(store, event);
        }
    }

    outcome
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{RegisterSession, Store};
    use nostr_sdk::prelude::{EventBuilder, Keys, Kind, Tag};

    fn make_tag(parts: &[&str]) -> Tag {
        Tag::parse(parts.iter().copied()).unwrap()
    }

    fn build_event(keys: &Keys, kind_n: u16, content: &str, tags: Vec<Tag>) -> nostr_sdk::Event {
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
                channel_h: channel_h.into(),
                child_pid: None,
                transcript_path: None,
                now: 1,
            })
            .unwrap();
    }

    #[test]
    fn chat_routes_to_channel_session_via_inbox_and_skips_sender() {
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
        let outcome = materialize(&RawEnvelope::Nostr(ambient.clone()), &store);
        assert!(
            !outcome.wake_mentions,
            "ambient message must not wake inbox"
        );
        assert!(store
            .peek_pending_for_pubkey(&receiver_pk)
            .unwrap()
            .is_empty());
        assert!(store.has_event(&ambient.id.to_hex()).unwrap());

        // Mention (p-tagged): routed to inbox and wakes doorbell.
        let mention = build_event(
            &sender_keys,
            9,
            "hey receiver, LGTM",
            vec![
                make_tag(&["h", "mychannel"]),
                make_tag(&["p", &receiver_pk]),
            ],
        );
        let outcome2 = materialize(&RawEnvelope::Nostr(mention.clone()), &store);
        assert!(outcome2.wake_mentions, "mention should wake inbox");
        let receiver_rows = store.peek_pending_for_pubkey(&receiver_pk).unwrap();
        assert_eq!(receiver_rows.len(), 1);
        assert_eq!(receiver_rows[0].body, "hey receiver, LGTM");
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
        assert!(!outcome.wake_mentions, "reaction must never wake mentions");
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
