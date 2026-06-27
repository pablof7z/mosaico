//! Fabric abstraction layer — Phase 4: materializer extracted from server.rs.
//!
//! Layering intent (see docs/fabric-architecture.md §Phase 3/4):
//!   Delivery    (subscribe, publish)  ← NostrDelivery
//!   WireCodec   (encode, decode)      ← Nip29WireCodec
//!   Materializer (store writes)       ← materialize()
//!   Transport                         ← (private detail of NostrDelivery)

pub(crate) mod group_management;
pub mod nip29;
pub mod nostr_delivery;
pub mod provider;
pub(crate) mod subscriptions;

/// Raw wire envelope crossing the transport boundary. Phase 3 adds only the
/// Nostr variant; additional transports (NMP, Marmot) add variants in Phase 5.
pub enum RawEnvelope {
    Nostr(nostr_sdk::Event),
}

/// Subscription scope that Delivery implementations convert into wire-level
/// filters. Transport-agnostic.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    pub authors: Vec<String>,
    pub project: Option<String>,
    pub mentions_to: Option<String>,
    pub owners: Vec<String>,
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
    use crate::fabric::nip29::materializer::Nip29Materializer;
    use crate::fabric::nip29::wire::Nip29WireCodec;

    let RawEnvelope::Nostr(event) = env;

    // NIP-29 group metadata cache (kind:39000, relay-authored).
    if event.kind.as_u16() == 39000 {
        Nip29Materializer::materialize_group_metadata(store, event);
        return MaterializationOutcome::default();
    }

    // NIP-29 admins snapshot (kind:39001, relay-authored).
    if event.kind.as_u16() == 39001 {
        Nip29Materializer::materialize_admins_snapshot(store, event);
        return MaterializationOutcome::default();
    }

    // NIP-29 membership snapshot (kind:39002, relay-authored).
    if event.kind.as_u16() == 39002 {
        Nip29Materializer::materialize_membership_snapshot(store, event);
        return MaterializationOutcome::default();
    }

    // Decode via the NIP-29 wire codec.
    let codec = Nip29WireCodec;
    let Some(de) = codec.decode(env) else {
        eprintln!(
            "[demux] kind:{} id:{} decode→None (no handler)",
            event.kind.as_u16(),
            &event.id.to_hex()[..8],
        );
        return MaterializationOutcome::default();
    };

    // Tail is sent for EVERY decoded event, including is_self (matches original).
    let mut outcome = MaterializationOutcome {
        tail: Some(de.clone()),
        wake_mentions: false,
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
            Nip29Materializer::materialize_profile(store, pf, now);
        }

        DomainEvent::Status(ref st) => {
            Nip29Materializer::materialize_status(store, st, event.created_at.as_secs(), now);
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

        // Routing is pubkey-based: event is signed with the sender's durable key
        // and the p-tag carries the receiver's durable pubkey. No session-derived
        // keys, no session-specific wire tags.
        let event = build_event(
            &sender_keys,
            9,
            "heads up: I pushed the parser fix",
            vec![
                make_tag(&["h", "myproject"]),
                make_tag(&["p", &receiver_pk]),
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
                    channel: String::new(),
                })
                .unwrap();
        }

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
