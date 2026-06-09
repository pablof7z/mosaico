//! Fabric abstraction layer — Phase 4: materializer extracted from server.rs.
//!
//! Layering intent (see docs/fabric-architecture.md §Phase 3/4):
//!   Delivery    (subscribe, publish)  ← NostrDelivery
//!   WireCodec   (encode, decode)      ← Kind1WireCodec
//!   Materializer (store writes)       ← materialize()
//!   Transport                         ← (private detail of NostrDelivery)

use crate::codec::SubScope;

pub mod kind1;
pub mod nip29;
pub mod nostr_delivery;

/// Raw wire envelope crossing the transport boundary. Phase 3 adds only the
/// Nostr variant; additional transports (NMP, Marmot) add variants in Phase 5.
pub enum RawEnvelope {
    Nostr(nostr_sdk::Event),
}

/// Subscription scope that Delivery implementations convert into wire-level
/// filters. Mirrors `codec::SubScope` but is transport-agnostic and will grow
/// (e.g. `thread`) without touching the legacy codec shim.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    pub authors: Vec<String>,
    pub project: Option<String>,
    pub mentions_to: Option<String>,
    pub owners: Vec<String>,
    /// Forward-looking: thread/conversation scope (unused this phase).
    pub thread: Option<String>,
}

impl From<&SubScope> for Scope {
    fn from(s: &SubScope) -> Self {
        Self {
            authors: s.authors.clone(),
            project: s.project.clone(),
            mentions_to: s.mentions_to.clone(),
            owners: s.owners.clone(),
            thread: None,
        }
    }
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
    /// Emitted for every successfully decoded event, including is_self.
    pub tail: Option<crate::domain::DomainEvent>,
    /// True when a mention was actually routed and waiters should be woken.
    pub wake_mentions: bool,
}

// ── Top-level dispatcher ──────────────────────────────────────────────────────

/// Decode one raw envelope and apply all store side-effects.
///
/// Reproduces `handle_incoming` EXACTLY, split across the nip29 and kind1
/// materializers. Observable behavior is unchanged: tail is emitted for every
/// decoded event (including is_self), and `wake_mentions` is set only when a
/// mention is actually routed.
///
/// ACL note: today every hosted-addressed mention is routed with no membership
/// gate — keep it that way.
/// TODO(phase 5/6): membership-gated admission + quarantine once
/// provider_instance/origins are hydrated.
pub fn materialize(
    env: &RawEnvelope,
    hosted: &[String],
    owners: &[String],
    now: u64,
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
            Kind1Materializer::materialize_profile(store, owners, pf, now);
        }

        DomainEvent::Presence(ref pr) => {
            Kind1Materializer::materialize_presence(store, pr, now);
        }

        DomainEvent::Status(ref st) => {
            Kind1Materializer::materialize_status(store, st, now);
        }

        DomainEvent::Mention(ref m) if hosted.contains(&m.to_pubkey) => {
            let to = m.to_pubkey.clone();
            let routed = Kind1Materializer::materialize_inbound_message(store, &to, m, event);
            if routed {
                outcome.wake_mentions = true;
            }
        }

        // Activity (always) and non-hosted Mention → no-op, matching original.
        _ => {}
    }

    outcome
}
