//! Local-only search over the canonical `messages` read model.
//!
//! Public identity and channel selectors are resolved by the daemon before this
//! query runs. This layer deals only in immutable pubkeys and opaque channel ids;
//! it never consults relay or membership state.

use super::{Message, MessageRecipient, Store};
use anyhow::Result;
use rusqlite::types::Value;

pub(crate) const MESSAGE_SEARCH_DEFAULT_LIMIT: u32 = 20;
pub(crate) const MESSAGE_SEARCH_MAX_LIMIT: u32 = 200;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MessageSearchQuery {
    /// Empty means every channel represented by a message row.
    pub(crate) channels: Vec<String>,
    pub(crate) from_pubkeys: Vec<String>,
    pub(crate) to_pubkeys: Vec<String>,
    pub(crate) contains: Vec<String>,
    pub(crate) since: Option<u64>,
    pub(crate) until: Option<u64>,
    pub(crate) limit: u32,
    pub(crate) before: Option<MessageSearchPosition>,
    /// The current daemon management key, needed even before its profile cache
    /// has warmed. Cached remote backend profiles are excluded independently.
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
    /// Search the canonical local message cache, globally newest first.
    ///
    /// Repeated values within a dimension are ORed. Different dimensions are
    /// ANDed. Text matching happens in Rust so literal substring semantics and
    /// Unicode case folding do not depend on SQLite's ASCII-only `lower()`.
    pub(crate) fn search_messages(&self, query: &MessageSearchQuery) -> Result<MessageSearchPage> {
        anyhow::ensure!(
            (1..=MESSAGE_SEARCH_MAX_LIMIT).contains(&query.limit),
            "message search limit must be between 1 and {MESSAGE_SEARCH_MAX_LIMIT}"
        );

        let mut clauses = vec![
            "NOT EXISTS (
                SELECT 1 FROM relay_profiles backend_author
                WHERE backend_author.pubkey=messages.author_pubkey
                  AND backend_author.is_backend=1
             )"
            .to_string(),
            "NOT EXISTS (
                SELECT 1
                FROM message_recipients backend_edge
                JOIN relay_profiles backend_recipient
                  ON backend_recipient.pubkey=backend_edge.recipient_pubkey
                WHERE backend_edge.message_id=messages.message_id
                  AND backend_recipient.is_backend=1
             )"
            .to_string(),
        ];
        let mut values = Vec::new();
        if let Some(backend_pubkey) = query
            .backend_pubkey
            .as_deref()
            .filter(|pubkey| !pubkey.is_empty())
        {
            clauses.push("author_pubkey<>?".to_string());
            values.push(Value::Text(backend_pubkey.to_string()));
            clauses.push(
                "NOT EXISTS (
                    SELECT 1 FROM message_recipients management_edge
                    WHERE management_edge.message_id=messages.message_id
                      AND management_edge.recipient_pubkey=?
                 )"
                .to_string(),
            );
            values.push(Value::Text(backend_pubkey.to_string()));
        }
        push_in_clause(&mut clauses, &mut values, "channel_h", &query.channels);
        push_in_clause(
            &mut clauses,
            &mut values,
            "author_pubkey",
            &query.from_pubkeys,
        );
        if !query.to_pubkeys.is_empty() {
            clauses.push(format!(
                "EXISTS (
                    SELECT 1 FROM message_recipients recipient
                    WHERE recipient.message_id=messages.message_id
                      AND recipient.recipient_pubkey IN ({})
                 )",
                placeholders(query.to_pubkeys.len())
            ));
            values.extend(query.to_pubkeys.iter().cloned().map(Value::Text));
        }
        if let Some(since) = query.since {
            clauses.push("created_at>=?".to_string());
            values.push(Value::Integer(u64_to_sql(since)?));
        }
        if let Some(until) = query.until {
            clauses.push("created_at<=?".to_string());
            values.push(Value::Integer(u64_to_sql(until)?));
        }
        if let Some(before) = &query.before {
            clauses.push("(created_at<? OR (created_at=? AND message_id<?))".to_string());
            let at = u64_to_sql(before.created_at)?;
            values.push(Value::Integer(at));
            values.push(Value::Integer(at));
            values.push(Value::Text(before.message_id.clone()));
        }

        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", clauses.join(" AND "))
        };
        let sql = format!(
            "SELECT {} FROM messages{}
             ORDER BY created_at DESC, message_id DESC",
            super::messages::MESSAGE_COLS,
            where_sql
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::params_from_iter(values.iter()),
            super::messages::row_to_message,
        )?;
        let needles = query
            .contains
            .iter()
            .map(|value| value.to_lowercase())
            .collect::<Vec<_>>();
        let wanted = query.limit as usize + 1;
        let mut matched = Vec::with_capacity(wanted);
        for row in rows {
            let message = row?;
            if !needles.is_empty() {
                let haystack = message.body.to_lowercase();
                if !needles.iter().any(|needle| haystack.contains(needle)) {
                    continue;
                }
            }
            matched.push(message);
            if matched.len() == wanted {
                break;
            }
        }

        let has_more = matched.len() > query.limit as usize;
        matched.truncate(query.limit as usize);
        let next = has_more.then(|| {
            let last = matched.last().expect("a page with more rows is non-empty");
            MessageSearchPosition {
                created_at: last.created_at,
                message_id: last.message_id.clone(),
            }
        });
        let hits = matched
            .into_iter()
            .map(|message| {
                let recipients = self.message_recipients(&message.message_id)?;
                Ok(MessageSearchHit {
                    message,
                    recipients,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(MessageSearchPage { hits, next })
    }

    /// Resolve one global public identity selector to an immutable pubkey.
    ///
    /// This intentionally reads only profile/handle caches. It does not scope by
    /// channel membership, running state, or caller identity.
    pub(crate) fn resolve_message_search_identity(&self, selector: &str) -> Result<String> {
        let selector = selector.trim().trim_start_matches('@');
        anyhow::ensure!(!selector.is_empty(), "identity selector must not be empty");
        if let Some(pubkey) = crate::idref::normalize_pubkey(selector) {
            return Ok(pubkey);
        }

        let mut matches = Vec::new();
        match crate::idref::parse_ref(selector) {
            crate::idref::Ref::Agent { slug, host } => {
                let mut stmt = self.conn.prepare(
                    "SELECT DISTINCT pubkey FROM relay_profiles
                     WHERE host=?1 AND (name=?2 OR slug=?2 OR agent_slug=?2)",
                )?;
                matches.extend(
                    stmt.query_map(rusqlite::params![host, slug], |row| row.get(0))?
                        .collect::<rusqlite::Result<Vec<String>>>()?,
                );
            }
            crate::idref::Ref::Token(token) => {
                let mut stmt = self.conn.prepare(
                    "SELECT DISTINCT pubkey FROM relay_profiles
                     WHERE name=?1 OR slug=?1 OR agent_slug=?1",
                )?;
                matches.extend(
                    stmt.query_map([&token], |row| row.get(0))?
                        .collect::<rusqlite::Result<Vec<String>>>()?,
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
            [] => anyhow::bail!("no cached identity matching {selector:?}"),
            _ => anyhow::bail!(
                "identity selector {selector:?} is ambiguous; use a full npub or pubkey"
            ),
        }
    }
}

fn push_in_clause(
    clauses: &mut Vec<String>,
    values: &mut Vec<Value>,
    column: &str,
    candidates: &[String],
) {
    if candidates.is_empty() {
        return;
    }
    clauses.push(format!("{column} IN ({})", placeholders(candidates.len())));
    values.extend(candidates.iter().cloned().map(Value::Text));
}

fn placeholders(len: usize) -> String {
    std::iter::repeat_n("?", len).collect::<Vec<_>>().join(",")
}

fn u64_to_sql(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| anyhow::anyhow!("timestamp exceeds SQLite integer range"))
}

#[cfg(test)]
#[path = "message_search/tests.rs"]
mod tests;
