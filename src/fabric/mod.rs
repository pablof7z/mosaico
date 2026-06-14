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

use crate::domain::Mention;

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
    /// True when a mention was actually routed and waiters should be woken.
    pub wake_mentions: bool,
    /// Canonical thread id an inbound message was filed under, when known.
    /// Tail consumers use this for exact thread attribution.
    pub thread_id: Option<String>,
}

// ── Top-level dispatcher ──────────────────────────────────────────────────────

/// Decode one raw envelope and apply all store side-effects.
///
/// Reproduces `handle_incoming` EXACTLY, split across the nip29 and kind1
/// materializers. Observable behavior is unchanged: tail is emitted for every
/// decoded event (including is_self), and `wake_mentions` is set only when a
/// mention is actually routed.
///
/// Routing gate for directed Mentions (to_pubkey ∈ hosted):
///   admitted = signer ∈ hosted  OR  signer ∈ owners  OR  is_group_member(project, signer)
/// The self-asserted `["agent", …]` wire tag carries no authority and is not
/// consulted here or anywhere; routing is by signer pubkey only.
///
/// Phase 6: `provider_instance` is threaded through to `materialize_inbound_message`
/// so canonical dual-write rows are keyed by the correct origin.
pub fn materialize(
    env: &RawEnvelope,
    hosted: &[String],
    owners: &[String],
    now: u64,
    provider_instance: &str,
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
        // is_self guard: skip store writes for own identity/presence/status.
        // Activity has no positive handler either (catch-all).
        DomainEvent::Profile(_)
        | DomainEvent::Presence(_)
        | DomainEvent::Activity(_)
        | DomainEvent::Status(_)
            if is_self => {}

        DomainEvent::Profile(ref pf) => {
            Kind1Materializer::materialize_profile(store, pf, now);
        }

        DomainEvent::Presence(ref pr) => {
            Kind1Materializer::materialize_presence(store, pr, now);
        }

        DomainEvent::Status(ref st) => {
            Kind1Materializer::materialize_status(store, st, now);
        }

        DomainEvent::Mention(ref m) if hosted.contains(&m.to_pubkey) => {
            // Admission gate: route only when the SIGNER is trusted.
            // admitted = signer ∈ hosted  OR  signer ∈ owners  OR
            //            is_group_member(project, signer)
            // The old self-asserted ["agent", …] wire tag is no longer consulted;
            // identity is the signer pubkey only.
            let sender_pk = event.pubkey.to_hex();
            let admitted = hosted.contains(&sender_pk)
                || owners.contains(&sender_pk)
                || store
                    .is_group_member(&m.project, &sender_pk)
                    .unwrap_or(false);
            if admitted {
                let to = m.to_pubkey.clone();
                // Slug is no longer on the wire; resolve from profiles/sessions table
                // so inbox rows carry a readable sender name for all senders.
                let resolved_slug = store
                    .resolve_slug_for_pubkey(&sender_pk)
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                let enriched = if resolved_slug.is_empty() {
                    std::borrow::Cow::Borrowed(m)
                } else {
                    std::borrow::Cow::Owned(Mention {
                        from: crate::domain::AgentRef::new(sender_pk, resolved_slug),
                        ..m.clone()
                    })
                };
                let (routed, thread_id) = Kind1Materializer::materialize_inbound_message(
                    store,
                    &to,
                    &enriched,
                    event,
                    provider_instance,
                    now,
                );
                if routed {
                    outcome.wake_mentions = true;
                }
                outcome.thread_id = thread_id;
                // Tail carries the enriched event so consumers see the slug.
                outcome.tail = Some(DomainEvent::Mention(enriched.into_owned()));
            }
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

    /// Owner-signed directed note (p + session-id + no agent) from a key in `owners`
    /// must route to the target session's inbox.
    #[test]
    fn owner_directed_note_routes_to_session_inbox() {
        let store = Store::open_memory().unwrap();
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let owner_pk = owner_keys.public_key().to_hex();
        let agent_pk = agent_keys.public_key().to_hex();
        let session_id = "test-sess-owner-1";

        // Register a live session for the agent so route_mention_into finds it.
        store
            .upsert_session(&crate::state::SessionRecord {
                session_id: session_id.to_string(),
                agent_slug: "claude".to_string(),
                agent_pubkey: agent_pk.clone(),
                project: "myproject".to_string(),
                host: "laptop".to_string(),
                child_pid: None,
                watch_pid: None,
                created_at: 1,
                alive: true,
                rel_cwd: String::new(),
            })
            .unwrap();
        store.touch_session(session_id, 1_000).unwrap();

        let event = build_event(
            &owner_keys,
            1,
            "looks good, ship it",
            vec![
                make_tag(&["h", "myproject"]),
                make_tag(&["p", &agent_pk]),
                make_tag(&["session-id", session_id]),
                // NO agent tag
            ],
        );

        let hosted = vec![agent_pk.clone()];
        let owners = vec![owner_pk.clone()];
        let env = RawEnvelope::Nostr(event);
        let outcome = materialize(&env, &hosted, &owners, 1_000, "test-pi", &store);

        assert!(outcome.wake_mentions, "owner-note must wake mentions");

        let inbox = store.drain_inbox(session_id).unwrap();
        assert_eq!(inbox.len(), 1, "one inbox row expected");
        assert_eq!(inbox[0].body, "looks good, ship it");
        assert_eq!(inbox[0].from_pubkey, owner_pk);
    }

    /// A directed note from a stranger (not in owners, not an agent) must NOT route.
    #[test]
    fn stranger_directed_note_does_not_route() {
        let store = Store::open_memory().unwrap();
        let stranger_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let agent_pk = agent_keys.public_key().to_hex();
        let owner_pk = owner_keys.public_key().to_hex();
        let session_id = "test-sess-stranger-1";

        store
            .upsert_session(&crate::state::SessionRecord {
                session_id: session_id.to_string(),
                agent_slug: "claude".to_string(),
                agent_pubkey: agent_pk.clone(),
                project: "myproject".to_string(),
                host: "laptop".to_string(),
                child_pid: None,
                watch_pid: None,
                created_at: 1,
                alive: true,
                rel_cwd: String::new(),
            })
            .unwrap();
        store.touch_session(session_id, 1_000).unwrap();

        let event = build_event(
            &stranger_keys,
            1,
            "I am a stranger",
            vec![
                make_tag(&["h", "myproject"]),
                make_tag(&["p", &agent_pk]),
                make_tag(&["session-id", session_id]),
                // NO agent tag, sender NOT in owners
            ],
        );

        let hosted = vec![agent_pk.clone()];
        let owners = vec![owner_pk]; // stranger is NOT in owners
        let env = RawEnvelope::Nostr(event);
        let outcome = materialize(&env, &hosted, &owners, 1_000, "test-pi", &store);

        assert!(
            !outcome.wake_mentions,
            "stranger note must NOT wake mentions"
        );

        let inbox = store.drain_inbox(session_id).unwrap();
        assert!(inbox.is_empty(), "inbox must be empty for stranger note");
    }

    /// Hosted-sender mention routes (signer ∈ hosted is the new gate).
    #[test]
    fn hosted_sender_mention_routes() {
        let store = Store::open_memory().unwrap();
        let sender_keys = Keys::generate();
        let recipient_keys = Keys::generate();
        let sender_pk = sender_keys.public_key().to_hex();
        let recipient_pk = recipient_keys.public_key().to_hex();
        let session_id = "test-sess-agent-1";

        store
            .upsert_session(&crate::state::SessionRecord {
                session_id: session_id.to_string(),
                agent_slug: "codex".to_string(),
                agent_pubkey: recipient_pk.clone(),
                project: "myproject".to_string(),
                host: "laptop".to_string(),
                child_pid: None,
                watch_pid: None,
                created_at: 1,
                alive: true,
                rel_cwd: String::new(),
            })
            .unwrap();
        store.touch_session(session_id, 1_000).unwrap();

        // Wire event has NO agent tag — the sender is hosted (same daemon).
        let event = build_event(
            &sender_keys,
            1,
            "hey review this",
            vec![
                make_tag(&["h", "myproject"]),
                make_tag(&["p", &recipient_pk]),
                make_tag(&["session-id", session_id]),
            ],
        );

        // Sender is in the hosted set — that is the admission criterion.
        let hosted = vec![recipient_pk.clone(), sender_pk.clone()];
        let owners: Vec<String> = vec![];
        let env = RawEnvelope::Nostr(event);
        let outcome = materialize(&env, &hosted, &owners, 1_000, "test-pi", &store);

        assert!(outcome.wake_mentions, "hosted-sender mention must route");
        let inbox = store.drain_inbox(session_id).unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].body, "hey review this");
    }

    /// Group-member sender mention routes (signer ∈ is_group_member).
    #[test]
    fn group_member_sender_mention_routes() {
        let store = Store::open_memory().unwrap();
        let sender_keys = Keys::generate();
        let recipient_keys = Keys::generate();
        let sender_pk = sender_keys.public_key().to_hex();
        let recipient_pk = recipient_keys.public_key().to_hex();
        let session_id = "test-sess-member-1";

        store
            .upsert_session(&crate::state::SessionRecord {
                session_id: session_id.to_string(),
                agent_slug: "claude".to_string(),
                agent_pubkey: recipient_pk.clone(),
                project: "myproject".to_string(),
                host: "laptop".to_string(),
                child_pid: None,
                watch_pid: None,
                created_at: 1,
                alive: true,
                rel_cwd: String::new(),
            })
            .unwrap();
        store.touch_session(session_id, 1_000).unwrap();

        // Register sender as a group member via the 39002 membership cache.
        store
            .replace_group_members(
                "myproject",
                &[(sender_pk.clone(), "member".to_string())],
                100,
            )
            .unwrap();

        let event = build_event(
            &sender_keys,
            1,
            "review please",
            vec![
                make_tag(&["h", "myproject"]),
                make_tag(&["p", &recipient_pk]),
                make_tag(&["session-id", session_id]),
            ],
        );

        // Sender is NOT hosted, NOT owner — admitted via group membership.
        let hosted = vec![recipient_pk.clone()];
        let owners: Vec<String> = vec![];
        let env = RawEnvelope::Nostr(event);
        let outcome = materialize(&env, &hosted, &owners, 1_000, "test-pi", &store);

        assert!(outcome.wake_mentions, "group-member mention must route");
        let inbox = store.drain_inbox(session_id).unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].body, "review please");
    }
}
