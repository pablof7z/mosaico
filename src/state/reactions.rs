//! NIP-25 reaction reads derived from the active NMP row view.

use super::*;
use std::collections::BTreeMap;

impl Store {
    pub fn reactions_on_authored_after(
        &self,
        author_pubkey: &str,
        since: u64,
        limit: u32,
    ) -> Result<Vec<ReactionRow>> {
        let messages = messages_by_id(&self.nmp_views.messages());
        let mut reactions = self
            .nmp_views
            .reactions()
            .into_iter()
            .filter_map(|reaction| {
                let message = messages.get(&reaction.target_message_id)?;
                (message.author_pubkey == author_pubkey
                    && reaction.created_at > since
                    && reaction.reactor_pubkey != author_pubkey)
                    .then(|| row(reaction, message.body.clone()))
            })
            .collect::<Vec<_>>();
        reactions.sort_by(|left, right| {
            (&left.created_at, &left.reaction_id).cmp(&(&right.created_at, &right.reaction_id))
        });
        reactions.truncate(limit as usize);
        Ok(reactions)
    }

    pub fn recent_reactions_for_channel(
        &self,
        channel_h: &str,
        since: u64,
        limit: u32,
    ) -> Result<Vec<ReactionRow>> {
        let messages = messages_by_id(&self.nmp_views.messages());
        let mut reactions = self
            .nmp_views
            .reactions()
            .into_iter()
            .filter(|reaction| reaction.channel_h == channel_h && reaction.created_at > since)
            .filter_map(|reaction| {
                let body = messages.get(&reaction.target_message_id)?.body.clone();
                Some(row(reaction, body))
            })
            .collect::<Vec<_>>();
        reactions.sort_by(|left, right| {
            (&right.created_at, &right.reaction_id).cmp(&(&left.created_at, &left.reaction_id))
        });
        reactions.truncate(limit as usize);
        Ok(reactions)
    }

    pub fn has_reaction_from_pubkey_on_message(
        &self,
        target_message_id: &str,
        reactor_pubkey: &str,
    ) -> Result<bool> {
        Ok(self.nmp_views.reactions().into_iter().any(|reaction| {
            reaction.target_message_id == target_message_id
                && reaction.reactor_pubkey == reactor_pubkey
        }))
    }
}

fn messages_by_id(messages: &[crate::nmp_views::MessageProjection]) -> BTreeMap<String, Message> {
    messages
        .iter()
        .map(|row| (row.message.message_id.clone(), row.message.clone()))
        .collect()
}

fn row(reaction: crate::nmp_views::ReactionProjection, target_body: String) -> ReactionRow {
    ReactionRow {
        reaction_id: reaction.reaction_id,
        target_message_id: reaction.target_message_id,
        channel_h: reaction.channel_h,
        reactor_pubkey: reaction.reactor_pubkey,
        emoji: reaction.emoji,
        created_at: reaction.created_at,
        target_body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{RelayEvent, TestRelayDelivery};

    fn event(id: &str, kind: u16, author: &str, content: &str, at: u64, tags: &str) -> RelayEvent {
        RelayEvent {
            id: id.to_string(),
            kind: kind as u32,
            pubkey: author.to_string(),
            created_at: at,
            channel_h: "chan".to_string(),
            d_tag: String::new(),
            content: content.to_string(),
            tags_json: tags.to_string(),
        }
    }

    #[test]
    fn authored_reactions_join_only_current_nmp_messages() {
        let store = Store::open_memory().unwrap();
        store.install_test_nmp_relay_delivery(TestRelayDelivery::new().events([
            event("mine", 9, "me", "pushed the fix", 5, "[]"),
            event("theirs", 9, "other", "unrelated", 5, "[]"),
            event("visible", 7, "peer", "👍", 20, r#"[["e","mine"]]"#),
            event("old", 7, "peer", "🎉", 8, r#"[["e","mine"]]"#),
            event("self", 7, "me", "✅", 20, r#"[["e","mine"]]"#),
            event("foreign", 7, "peer", "👀", 20, r#"[["e","theirs"]]"#),
        ]));

        let rows = store.reactions_on_authored_after("me", 10, 50).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].reaction_id, "visible");
        assert_eq!(rows[0].target_body, "pushed the fix");

        store.install_test_nmp_relay_delivery(TestRelayDelivery::new());
        assert!(store
            .reactions_on_authored_after("me", 0, 50)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn recent_channel_reactions_keep_newest_valid_rows() {
        let store = Store::open_memory().unwrap();
        store.install_test_nmp_relay_delivery(TestRelayDelivery::new().events([
            event("target", 9, "author", "decision", 1, "[]"),
            event("old", 7, "peer", "👍", 10, r#"[["e","target"]]"#),
            event(
                "invalid",
                7,
                "peer",
                "not an emoji",
                40,
                r#"[["e","target"]]"#,
            ),
            event("middle", 7, "peer", "👍", 20, r#"[["e","target"]]"#),
            event("new", 7, "peer", "👍", 30, r#"[["e","target"]]"#),
        ]));

        let rows = store.recent_reactions_for_channel("chan", 0, 2).unwrap();
        assert_eq!(
            rows.iter()
                .map(|reaction| reaction.reaction_id.as_str())
                .collect::<Vec<_>>(),
            ["new", "middle"]
        );
    }
}
