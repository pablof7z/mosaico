use super::Nip29Materializer;
use crate::domain::Reaction;
use crate::state::Store;
use nostr::Event;

impl Nip29Materializer {
    /// Materialise a decoded kind:7 reaction into `relay_reactions` ONLY. A
    /// reaction is passive awareness: it writes no `inbox` row and no
    /// `message_recipients` edge, so no live-delivery/doorbell path can ever pick
    /// it up. Idempotent by the reaction event id (a relay echo collapses onto the
    /// same row).
    pub fn materialize_reaction(store: &Store, event: &Event, rx: &Reaction) {
        let reaction_id = event.id.to_hex();
        if let Err(e) = store.upsert_reaction(
            &reaction_id,
            &rx.target_event_id,
            &rx.channel,
            &event.pubkey.to_hex(),
            &rx.emoji,
            event.created_at.as_secs(),
        ) {
            tracing::error!(
                reaction_id = %reaction_id,
                target = %rx.target_event_id,
                error = %e,
                "materialize_reaction: relay_reactions upsert failed — relay truth diverged from cache"
            );
        }
    }
}
