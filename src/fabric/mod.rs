//! Fabric abstraction layer — Phase 3: raw Nostr delivery extracted from codec.
//!
//! Layering intent (see docs/fabric-architecture.md §Phase 3):
//!   Delivery  (subscribe, publish)  ← NostrDelivery
//!   WireCodec (encode, decode)      ← Kind1WireCodec
//!   Transport                       ← (private detail of NostrDelivery)

use crate::codec::SubScope;

pub mod kind1;
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
