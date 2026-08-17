use super::Nip29Materializer;
use crate::domain::Reaction;
use crate::fabric::ProjectionProvenance;
use crate::state::{ProjectionKind, Store};
use nostr::Event;

impl Nip29Materializer {
    /// Materialise a decoded kind:7 reaction into `relay_reactions` ONLY. A
    /// reaction is passive awareness: it writes no `inbox` row and no
    /// `message_recipients` edge, so no live-delivery/doorbell path can ever pick
    /// it up. Idempotent by the reaction event id (a relay echo collapses onto the
    /// same row).
    pub(crate) fn materialize_reaction(
        store: &Store,
        event: &Event,
        rx: &Reaction,
        provenance: &ProjectionProvenance,
    ) {
        let reaction_id = event.id.to_hex();
        let projected = store
            .upsert_reaction(
                &reaction_id,
                &rx.target_event_id,
                &rx.channel,
                &event.pubkey.to_hex(),
                &rx.emoji,
                event.created_at.as_secs(),
            )
            .and_then(|_| {
                store.set_projection_source(
                    ProjectionKind::Reaction,
                    &reaction_id,
                    &provenance.source_event_id,
                )
            });
        if let Err(e) = projected {
            tracing::error!(
                reaction_id = %reaction_id,
                target = %rx.target_event_id,
                error = %e,
                "materialize_reaction: relay_reactions upsert failed — relay truth diverged from cache"
            );
        }
    }
}
