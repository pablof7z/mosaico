//! `Kind1WireCodec` — `WireCodec` implementation that delegates to the existing
//! `crate::codec::kind1::Kind1Codec` for encode and decode. Keeps the old codec
//! as the single source of truth for wire-shape logic until Phase 4 moves it.

use crate::codec::kind1::Kind1Codec;
use crate::codec::Codec as LegacyCodec;
use crate::fabric::{RawEnvelope, WireCodec};
use anyhow::Result;

pub struct Kind1WireCodec;

impl WireCodec for Kind1WireCodec {
    fn encode(&self, ev: &crate::domain::DomainEvent) -> Result<nostr_sdk::EventBuilder> {
        Kind1Codec.encode(ev)
    }

    fn decode(&self, env: &RawEnvelope) -> Option<crate::domain::DomainEvent> {
        match env {
            RawEnvelope::Nostr(event) => Kind1Codec.decode(event),
        }
    }
}
