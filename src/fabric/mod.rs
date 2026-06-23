//! Fabric abstraction layer — Phase 4: materializer extracted from server.rs.
//!
//! Layering intent (see docs/fabric-architecture.md §Phase 3/4):
//!   Delivery    (subscribe, publish)  ← NostrDelivery
//!   WireCodec   (encode, decode)      ← Kind1WireCodec
//!   Materializer (store writes)       ← materialize()
//!   Transport                         ← (private detail of NostrDelivery)

pub mod kind1;
pub mod nip29;
pub mod nostr_delivery;
pub mod provider;

/// Raw wire envelope crossing the transport boundary. Phase 3 adds only the
/// Nostr variant; additional transports (NMP, Marmot) add variants in Phase 5.
pub enum RawEnvelope {
    Nostr(nostr_sdk::Event),
}

/// Subscription scope that Delivery implementations convert into wire-level
/// filters. Transport-agnostic; will grow (e.g. `thread`) without touching
/// the legacy codec layer.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    pub authors: Vec<String>,
    pub project: Option<String>,
    pub mentions_to: Option<String>,
    pub owners: Vec<String>,
    /// Forward-looking: thread/conversation scope (unused this phase).
    pub thread: Option<String>,
}

/// Encode/decode between `DomainEvent` and `RawEnvelope`. Transport-agnostic.
pub trait WireCodec {
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
    /// Canonical thread id an inbound message was filed under, when known.
    /// Tail consumers use this for exact thread attribution.
    pub thread_id: Option<String>,
}

// ── Top-level dispatcher ──────────────────────────────────────────────────────

/// Decode one raw envelope and apply all store side-effects.
///
/// Tail is emitted for every decoded event (including is_self).
/// `wake_mentions` is set only when a chat message is routed to a live session.
///
pub fn materialize(
    env: &RawEnvelope,
    hosted: &[String],
    now: u64,
    _provider_instance: &str,
    store: &crate::state::Store,
) -> MaterializationOutcome {
    use crate::domain::DomainEvent;
    use crate::fabric::kind1::materializer::Kind1Materializer;
    use crate::fabric::kind1::wire::Kind1WireCodec;
    use crate::fabric::nip29::materializer::Nip29Materializer;

    let RawEnvelope::Nostr(event) = env;

    // NIP-29 group metadata cache (kind:39000, relay-authored).
    if event.kind.as_u16() == 39000 {
        Nip29Materializer::materialize_group_metadata(store, event);
        return MaterializationOutcome::default();
    }

    // NIP-29 membership snapshot (kind:39002, relay-authored).
    if event.kind.as_u16() == 39002 {
        Nip29Materializer::materialize_membership_snapshot(store, event);
        return MaterializationOutcome::default();
    }

    // Decode via the Kind1 wire codec.
    let codec = Kind1WireCodec;
    let Some(de) = codec.decode(env) else {
        return MaterializationOutcome::default();
    };

    // Tail is sent for EVERY decoded event, including is_self (matches original).
    let mut outcome = MaterializationOutcome {
        tail: Some(de.clone()),
        wake_mentions: false,
        thread_id: None,
    };

    let is_self = hosted.contains(&event.pubkey.to_hex());

    match de {
        // is_self guard: skip materializing our OWN kind:0 profile echo (local
        // identity is authoritative). The guard is no longer needed for Status:
        // `materialize_status` writes ONLY to `peer_session_state`, never the
        // authoritative `session_state`, so a self-status echo cannot corrupt
        // local truth. Activity has no positive handler either way (catch-all).
        DomainEvent::Profile(_) if is_self => {}

        DomainEvent::Profile(ref pf) => {
            Kind1Materializer::materialize_profile(store, pf, now);
        }

        DomainEvent::Status(ref st) => {
            Kind1Materializer::materialize_status(store, st, event.created_at.as_secs(), now);
        }

        DomainEvent::ChatMessage(ref chat) => {
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
            if Nip29Materializer::materialize_chat_message(store, &enriched, event) {
                outcome.wake_mentions = true;
            }
            outcome.tail = Some(DomainEvent::ChatMessage(enriched.into_owned()));
        }

        // Activity (always) and non-hosted Mention → no-op, matching original.
        _ => {}
    }

    outcome
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Store;
    use nostr_sdk::prelude::{EventBuilder, Keys, Kind, Tag};

    fn make_tag(parts: &[&str]) -> Tag {
        Tag::parse(parts.iter().copied()).unwrap()
    }

    /// Build and sign a raw kind:1 event using the given keys and tags.
    fn build_event(keys: &Keys, kind_n: u16, content: &str, tags: Vec<Tag>) -> nostr_sdk::Event {
        EventBuilder::new(Kind::from(kind_n), content)
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }

    #[test]
    fn chat_message_routes_only_to_sessions_alive_at_event_time() {
        let store = Store::open_memory().unwrap();
        let sender_keys = Keys::generate();
        let receiver_keys = Keys::generate();
        let future_keys = Keys::generate();
        let sender_pk = sender_keys.public_key().to_hex();
        let receiver_pk = receiver_keys.public_key().to_hex();
        let future_pk = future_keys.public_key().to_hex();

        // Stage 4: session keys are the wire identity; session_pubkey_info
        // derives from_session / mentioned_session for routing and DB rows.
        let sender_sess_keys = Keys::generate();
        let receiver_sess_keys = Keys::generate();
        let sender_sess_pk = sender_sess_keys.public_key().to_hex();
        let receiver_sess_pk = receiver_sess_keys.public_key().to_hex();

        // Sign with sender's SESSION key; p-tag carries receiver's SESSION pubkey.
        // No from-session / session-id tags — Stage 4 drops them from the wire.
        let event = build_event(
            &sender_sess_keys,
            9,
            "heads up: I pushed the parser fix",
            vec![
                make_tag(&["h", "myproject"]),
                make_tag(&["p", &receiver_sess_pk]),
            ],
        );
        let event_ts = event.created_at.as_secs();

        for (session_id, slug, pubkey, created_at) in [
            ("sender-sess", "sender", sender_pk.clone(), 1),
            ("receiver-sess", "receiver", receiver_pk.clone(), 1),
            ("future-sess", "future", future_pk.clone(), event_ts + 1),
        ] {
            store
                .upsert_session(&crate::state::SessionRecord {
                    session_id: session_id.to_string(),
                    agent_slug: slug.to_string(),
                    agent_pubkey: pubkey,
                    project: "myproject".to_string(),
                    host: "laptop".to_string(),
                    child_pid: None,
                    watch_pid: None,
                    created_at,
                    alive: true,
                    rel_cwd: String::new(),
                })
                .unwrap();
        }

        // Register session pubkeys so from_session / mentioned_session can be
        // derived via session_pubkey_info during routing.
        store
            .upsert_session_pubkey(&sender_sess_pk, "sender-sess", &sender_pk, "sender", 1)
            .unwrap();
        store
            .upsert_session_pubkey(&receiver_sess_pk, "receiver-sess", &receiver_pk, "receiver", 1)
            .unwrap();

        let hosted = vec![sender_pk, receiver_pk, future_pk];
        let env = RawEnvelope::Nostr(event);
        let outcome = materialize(&env, &hosted, event_ts, "test-pi", &store);

        assert!(outcome.wake_mentions, "live receiver should wake");
        assert!(
            store.drain_chat("sender-sess").unwrap().is_empty(),
            "sender session should not receive its own chat line"
        );
        let receiver_rows = store.drain_chat("receiver-sess").unwrap();
        assert_eq!(receiver_rows.len(), 1);
        assert_eq!(receiver_rows[0].body, "heads up: I pushed the parser fix");
        assert_eq!(receiver_rows[0].mentioned_session, "receiver-sess");
        assert!(
            store.drain_chat("future-sess").unwrap().is_empty(),
            "sessions created after the event must not receive chat backfill"
        );
    }

}
