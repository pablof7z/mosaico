//! Fabric abstraction layer — Phase 4: materializer extracted from server.rs.
//!
//! Layering intent (see docs/fabric-architecture.md §Phase 3/4):
//!   Delivery    (subscribe, publish)  ← NostrDelivery
//!   NostrEventCodec (encode, decode)  ← Nip29WireCodec
//!   Materializer (store writes)       ← materialize()
//!   Transport                         ← (private detail of NostrDelivery)

pub(crate) mod group_management;
pub mod nip29;
pub mod nostr_delivery;
pub mod provider;
pub(crate) mod subscriptions;

/// Raw envelope currently emitted by the Nostr delivery path.
///
/// This is intentionally not advertised as transport-neutral: current materialization
/// is NIP-29-over-Nostr-specific. Future providers should own their native
/// envelope type instead of making this enum a cross-fabric dumping ground.
pub enum RawEnvelope {
    Nostr(nostr_sdk::Event),
}

/// Subscription scope that Delivery implementations convert into wire-level
/// filters. Transport-agnostic.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    pub authors: Vec<String>,
    pub project: Option<String>,
}

/// Encode/decode between `DomainEvent` and Nostr event envelopes.
///
/// The return type is `nostr_sdk::EventBuilder`, so this boundary is explicitly
/// Nostr-specific even when a concrete codec maps NIP-29 group semantics.
pub trait NostrEventCodec {
    fn encode(&self, ev: &crate::domain::DomainEvent) -> anyhow::Result<nostr_sdk::EventBuilder>;
    fn decode(&self, env: &RawEnvelope) -> Option<crate::domain::DomainEvent>;
}

/// Shell trait for Delivery implementations — subscribe is inherent on
/// `NostrDelivery` (avoids async-fn-in-trait / async_trait machinery).
/// Full trait surface (publish, fetch, notifications, etc.) is Phase 5.
pub trait Delivery {
    fn name(&self) -> &'static str;
}

// ── Materializer output ───────────────────────────────────────────────────────

/// The two side-effects that `handle_incoming` performs outside the store.
/// All other effects (quarantine, sync-state reconciliation) are reserved for
/// later phases.
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
/// Every observed event is materialized into exactly one cache by kind:
///   * 39000 → relay_channels, 39001/39002 → relay_channel_members,
///   * 0 → relay_profiles, 30315 → relay_status,
///   * every other kind → relay_events (verbatim log, NIP-01 replacement).
///
/// Chat (kind:9) is additionally routed into the inbox ledger for local sessions
/// in its channel. The same ledger stores per-target orchestration claims.
///
/// Tail is emitted for every decoded domain event. `wake_mentions` is set only
/// when a chat message is newly routed to a live local session.
///
/// `_hosted` and `_now` are retained for call-site compatibility: caches now key
/// off the event's own `created_at` (NIP-01 newest-wins) and read identically for
/// local and remote agents, so neither the hosted set nor wall-clock `now` gate
/// materialization.
pub fn materialize(
    env: &RawEnvelope,
    _hosted: &[String],
    _now: u64,
    _provider_instance: &str,
    store: &crate::state::Store,
) -> MaterializationOutcome {
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

    // Decode via the NIP-29 wire codec. Kinds the codec does not recognise are
    // still cached verbatim in relay_events (e.g. reactions, foreign kinds) —
    // EXCEPT the dedicated-cache kinds (0, 30315), which must never land in the
    // verbatim log. A kind:30315 that fails to decode is simply dropped rather
    // than cached as a generic event.
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
            // Cache the chat line in the verbatim log, then route it into the
            // inbox ledger for the local sessions occupying its channel.
            Nip29Materializer::materialize_event(store, event);

            let sender_pk = event.pubkey.to_hex();
            let resolved_slug = store
                .resolve_slug_for_pubkey(&sender_pk)
                .ok()
                .flatten()
                .unwrap_or_default();
            let enriched = if resolved_slug.is_empty() {
                std::borrow::Cow::Borrowed(chat)
            } else {
                std::borrow::Cow::Owned(crate::domain::ChatMessage {
                    from: crate::domain::AgentRef::new(sender_pk, resolved_slug),
                    ..chat.clone()
                })
            };
            outcome.wake_mentions = Nip29Materializer::route_chat(store, event, &enriched);
            outcome.tail = Some(DomainEvent::ChatMessage(enriched.into_owned()));
        }

        // Activity (kind:1) and Proposal (kind:30023) carry no inbox routing; they
        // are cached verbatim in relay_events alongside every other kind.
        DomainEvent::Activity(_) | DomainEvent::Proposal(_) => {
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

    fn register(store: &Store, pubkey: &str, channel_h: &str, external_id: &str) -> String {
        store
            .register_session(&RegisterSession {
                harness: "claude-code".into(),
                external_id_kind: "harness_session".into(),
                external_id: external_id.into(),
                agent_pubkey: pubkey.into(),
                agent_slug: "agent".into(),
                channel_h: channel_h.into(),
                child_pid: None,
                transcript_path: None,
                resume_id: String::new(),
                now: 1,
            })
            .unwrap()
    }

    #[test]
    fn chat_routes_to_channel_session_via_inbox_and_skips_sender() {
        let store = Store::open_memory().unwrap();
        let sender_keys = Keys::generate();
        let receiver_keys = Keys::generate();
        let sender_pk = sender_keys.public_key().to_hex();
        let receiver_pk = receiver_keys.public_key().to_hex();

        let sender_sid = register(&store, &sender_pk, "myproject", "sender-ext");
        let receiver_sid = register(&store, &receiver_pk, "myproject", "receiver-ext");

        // Ambient message (no p-tag): stored in relay_events, inbox stays empty.
        let ambient = build_event(
            &sender_keys,
            9,
            "heads up: I pushed the parser fix",
            vec![make_tag(&["h", "myproject"])],
        );
        let ambient_ts = ambient.created_at.as_secs();
        let hosted = vec![sender_pk.clone(), receiver_pk.clone()];
        let outcome = materialize(
            &RawEnvelope::Nostr(ambient.clone()),
            &hosted,
            ambient_ts,
            "test-pi",
            &store,
        );
        assert!(
            !outcome.wake_mentions,
            "ambient message must not wake inbox"
        );
        assert!(store
            .peek_pending_for_session(&receiver_sid)
            .unwrap()
            .is_empty());
        assert!(store.has_event(&ambient.id.to_hex()).unwrap());

        // Mention (p-tagged): routed to inbox and wakes doorbell.
        let mention = build_event(
            &sender_keys,
            9,
            "hey receiver, LGTM",
            vec![
                make_tag(&["h", "myproject"]),
                make_tag(&["p", &receiver_pk]),
            ],
        );
        let mention_ts = mention.created_at.as_secs();
        let outcome2 = materialize(
            &RawEnvelope::Nostr(mention.clone()),
            &hosted,
            mention_ts,
            "test-pi",
            &store,
        );
        assert!(outcome2.wake_mentions, "mention should wake inbox");
        let receiver_rows = store.peek_pending_for_session(&receiver_sid).unwrap();
        assert_eq!(receiver_rows.len(), 1);
        assert_eq!(receiver_rows[0].body, "hey receiver, LGTM");
        assert!(
            store
                .peek_pending_for_session(&sender_sid)
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
            vec![make_tag(&["d", "proj"]), make_tag(&["name", "Project"])],
        );
        let env = RawEnvelope::Nostr(event);
        let outcome = materialize(&env, &[], 0, "test-pi", &store);
        assert!(outcome.tail.is_none(), "relay-authored state has no tail");
        assert_eq!(store.get_channel("proj").unwrap().unwrap().name, "Project");
    }

    #[test]
    fn unknown_kind_is_cached_verbatim() {
        let store = Store::open_memory().unwrap();
        let agent = Keys::generate();
        // kind:7 (reaction) is not decoded by the codec but must still be cached.
        let event = build_event(&agent, 7, "+", vec![make_tag(&["h", "proj"])]);
        let env = RawEnvelope::Nostr(event.clone());
        materialize(&env, &[], 0, "test-pi", &store);
        assert!(store.has_event(&event.id.to_hex()).unwrap());
    }
}
