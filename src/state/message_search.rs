//! Local search over the current NIP-29 kind:9 projection.

use super::{Message, MessageRecipient, Store};
use anyhow::Result;
use std::collections::BTreeSet;

pub(crate) const MESSAGE_SEARCH_DEFAULT_LIMIT: u32 = 20;
pub(crate) const MESSAGE_SEARCH_MAX_LIMIT: u32 = 200;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MessageSearchQuery {
    pub(crate) channels: Vec<String>,
    pub(crate) from_pubkeys: Vec<String>,
    pub(crate) to_pubkeys: Vec<String>,
    pub(crate) contains: Vec<String>,
    pub(crate) since: Option<u64>,
    pub(crate) until: Option<u64>,
    pub(crate) limit: u32,
    pub(crate) before: Option<MessageSearchPosition>,
    pub(crate) backend_pubkey: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MessageSearchPosition {
    pub(crate) created_at: u64,
    pub(crate) message_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MessageSearchHit {
    pub(crate) message: Message,
    pub(crate) recipients: Vec<MessageRecipient>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MessageSearchPage {
    pub(crate) hits: Vec<MessageSearchHit>,
    pub(crate) next: Option<MessageSearchPosition>,
}

impl Store {
    /// Search current Nostr runtime messages, newest first. Values within one dimension
    /// are ORed; dimensions are ANDed.
    pub(crate) fn search_messages(&self, query: &MessageSearchQuery) -> Result<MessageSearchPage> {
        anyhow::ensure!(
            (1..=MESSAGE_SEARCH_MAX_LIMIT).contains(&query.limit),
            "message search limit must be between 1 and {MESSAGE_SEARCH_MAX_LIMIT}"
        );

        let view = self.view();
        let backend_pubkeys = view
            .profiles()
            .iter()
            .filter(|profile| profile.is_backend)
            .map(|profile| profile.pubkey.clone())
            .chain(
                query
                    .backend_pubkey
                    .iter()
                    .filter(|pubkey| !pubkey.is_empty())
                    .cloned(),
            )
            .collect::<BTreeSet<_>>();
        let needles = query
            .contains
            .iter()
            .map(|value| value.to_lowercase())
            .collect::<Vec<_>>();
        let mut hits = view
            .messages()
            .iter()
            .filter(|row| {
                let (message, recipients) = (&row.message, &row.recipients);
                !backend_pubkeys.contains(&message.author_pubkey)
                    && !recipients
                        .iter()
                        .any(|recipient| backend_pubkeys.contains(&recipient.recipient_pubkey))
                    && matches_any(&query.channels, &message.channel_h)
                    && matches_any(&query.from_pubkeys, &message.author_pubkey)
                    && (query.to_pubkeys.is_empty()
                        || recipients.iter().any(|recipient| {
                            query.to_pubkeys.contains(&recipient.recipient_pubkey)
                        }))
                    && query.since.is_none_or(|since| message.created_at >= since)
                    && query.until.is_none_or(|until| message.created_at <= until)
                    && query.before.as_ref().is_none_or(|before| {
                        (message.created_at, message.message_id.as_str())
                            < (before.created_at, before.message_id.as_str())
                    })
                    && (needles.is_empty()
                        || needles
                            .iter()
                            .any(|needle| message.body.to_lowercase().contains(needle)))
            })
            .collect::<Vec<_>>();
        hits.sort_by(|left, right| {
            (&right.message.created_at, &right.message.message_id)
                .cmp(&(&left.message.created_at, &left.message.message_id))
        });

        let has_more = hits.len() > query.limit as usize;
        hits.truncate(query.limit as usize);
        // Only the returned page reads local attachment directories.
        let hits = hits
            .into_iter()
            .map(|row| {
                Ok(MessageSearchHit {
                    message: self.with_local_attachment(row.message.clone())?,
                    recipients: row.recipients.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let next = has_more.then(|| {
            let last = hits.last().expect("a page with more rows is non-empty");
            MessageSearchPosition {
                created_at: last.message.created_at,
                message_id: last.message.message_id.clone(),
            }
        });
        Ok(MessageSearchPage { hits, next })
    }

    /// Resolve a public identity from the current Nostr runtime profile delivery plus
    /// product-local handle leases.
    pub(crate) fn resolve_message_search_identity(&self, selector: &str) -> Result<String> {
        let selector = selector.trim().trim_start_matches('@');
        anyhow::ensure!(!selector.is_empty(), "identity selector must not be empty");
        if let Some(pubkey) = crate::idref::normalize_pubkey(selector) {
            return Ok(pubkey);
        }

        let mut matches = Vec::new();
        match crate::idref::parse_ref(selector) {
            crate::idref::Ref::Agent { slug, host } => {
                matches.extend(
                    self.view()
                        .profiles()
                        .iter()
                        .filter(|profile| {
                            profile.host == host
                                && (profile.name == slug
                                    || profile.slug == slug
                                    || profile.agent_slug == slug)
                        })
                        .map(|profile| profile.pubkey.clone()),
                );
            }
            crate::idref::Ref::Token(token) => {
                matches.extend(
                    self.view()
                        .profiles()
                        .iter()
                        .filter(|profile| {
                            profile.name == token
                                || profile.slug == token
                                || profile.agent_slug == token
                        })
                        .map(|profile| profile.pubkey.clone()),
                );
                if let Some(pubkey) = self.pubkey_for_handle(&token)? {
                    matches.push(pubkey);
                }
            }
            crate::idref::Ref::Pubkey(_) => unreachable!("normalized above"),
        }
        matches.sort();
        matches.dedup();
        match matches.as_slice() {
            [pubkey] => Ok(pubkey.clone()),
            [] => anyhow::bail!("no observed identity matching {selector:?}"),
            _ => anyhow::bail!(
                "identity selector {selector:?} is ambiguous; use a full npub or pubkey"
            ),
        }
    }
}

fn matches_any(candidates: &[String], actual: &str) -> bool {
    candidates.is_empty() || candidates.iter().any(|candidate| candidate == actual)
}

#[cfg(test)]
#[path = "message_search/tests.rs"]
mod tests;
