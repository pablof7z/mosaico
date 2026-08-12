use crate::domain::DomainEvent;
#[cfg(test)]
use crate::domain::Reaction;
use crate::fabric::nip29::wire::{Nip29WireCodec, KIND_REACTION};

use super::NmpViews;

#[derive(Clone)]
pub(crate) struct ReactionProjection {
    pub(crate) reaction_id: String,
    pub(crate) target_message_id: String,
    pub(crate) channel_h: String,
    pub(crate) reactor_pubkey: String,
    pub(crate) emoji: String,
    pub(crate) created_at: u64,
}

impl NmpViews {
    pub(crate) fn reactions(&self) -> Vec<ReactionProjection> {
        #[cfg(test)]
        if self.test_relay_delivery().is_some() {
            return self
                .projected_events_for_kind(KIND_REACTION)
                .into_iter()
                .filter_map(|row| {
                    let event = row.event;
                    let target_message_id =
                        super::messages::tag_values(&super::messages::tags(&event.tags_json), "e")
                            .into_iter()
                            .next()?;
                    Reaction::emoji_is_valid(&event.content).then_some(ReactionProjection {
                        reaction_id: event.id,
                        target_message_id,
                        channel_h: event.channel_h,
                        reactor_pubkey: event.pubkey,
                        emoji: event.content,
                        created_at: event.created_at,
                    })
                })
                .collect();
        }

        self.rows_for_kind(KIND_REACTION)
            .into_iter()
            .filter_map(|row| {
                let DomainEvent::Reaction(reaction) =
                    Nip29WireCodec.decode_event(&row.row.event)?
                else {
                    return None;
                };
                Some(ReactionProjection {
                    reaction_id: row.row.event.id.to_hex(),
                    target_message_id: reaction.target_event_id,
                    channel_h: reaction.channel,
                    reactor_pubkey: reaction.reactor.pubkey,
                    emoji: reaction.emoji,
                    created_at: row.row.event.created_at.as_secs(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use nmp::{Row, RowDelta};
    use nostr::{EventBuilder, Keys, Kind, Tag};

    use super::*;

    fn reaction() -> Row {
        Row {
            event: EventBuilder::new(Kind::from(KIND_REACTION), "👍")
                .tags([
                    Tag::parse(["h", "room"]).unwrap(),
                    Tag::parse(["e", "target"]).unwrap(),
                ])
                .sign_with_keys(&Keys::generate())
                .unwrap(),
            sources: BTreeSet::new(),
        }
    }

    #[test]
    fn removed_nmp_row_removes_the_reaction_projection() {
        let views = NmpViews::default();
        let row = reaction();
        let id = row.event.id;

        views.apply_frame("reactions", 1, vec![RowDelta::Added(row)], vec![]);
        assert_eq!(views.reactions()[0].target_message_id, "target");

        views.apply_frame("reactions", 1, vec![RowDelta::Removed(id)], vec![]);
        assert!(views.reactions().is_empty());
    }
}
