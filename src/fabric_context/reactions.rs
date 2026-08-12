//! Reaction awareness: the delta-scoped "reactions on your own recent messages"
//! section. The store read ([`capture_reaction_sources`]) freezes the inputs;
//! [`group_reactions`] then derives the rendered rows.
//!
//! A reaction is passive awareness only — it comes from a round-tripped kind:7
//! delivered by NMP and is surfaced here at turn start. It never enters the
//! mention/inbox path.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::messages::is_backend_pubkey;
use super::model::ReactionRow;
use super::refs::pubkey_ref;
use crate::state::Store;
use crate::util::relative_time;

/// Widest capture cap; grouping re-applies the real render cap.
const REACTION_CAPTURE_CAP: u32 = 1_000;
/// Maximum reaction groups rendered per turn (token budget).
const MAX_REACTION_ROWS: usize = 8;
/// Snippet length for the reacted-to message body.
const SNIPPET_CHARS: usize = 50;

/// A pre-resolved reaction source row (now/cursor-independent). Grouping applies
/// the cursor delta and the render cap.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ReactionCap {
    pub(super) reactor_ref: String,
    pub(super) emoji: String,
    pub(super) target_message_id: String,
    pub(super) target_snippet: String,
    pub(super) created_at: u64,
}

/// Read the reactions on `self_pubkey`'s authored messages since `since`, dropping
/// backend (e.g. daemon 👁 receipt) reactors, and resolve display refs + snippets.
/// `since` is a session-stable floor (session creation time), NOT the cursor — the
/// cursor delta is applied later by [`group_reactions`], keeping this read a
/// cursor-independent superset.
pub(super) fn capture_reaction_sources(
    store: &Store,
    self_pubkey: &str,
    since: u64,
    local_host: &str,
    backend_pubkey: &str,
) -> Vec<ReactionCap> {
    if self_pubkey.is_empty() {
        return Vec::new();
    }
    store
        .reactions_on_authored_after(self_pubkey, since, REACTION_CAPTURE_CAP)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| !is_backend_pubkey(store, backend_pubkey, &r.reactor_pubkey))
        .map(|r| ReactionCap {
            reactor_ref: pubkey_ref(store, &r.reactor_pubkey, local_host),
            emoji: r.emoji,
            target_message_id: r.target_message_id,
            target_snippet: snippet(&r.target_body),
            created_at: r.created_at,
        })
        .collect()
}

/// Group reaction sources by `(target message, emoji)`, apply the cursor delta so
/// each reaction shows once, sort oldest-first, and cap. Returns the rendered rows
/// plus the count elided by the cap.
pub(super) fn group_reactions(
    sources: &[ReactionCap],
    cursor: u64,
    now: u64,
) -> (Vec<ReactionRow>, usize) {
    struct Group {
        reactors: Vec<String>,
        emoji: String,
        target_snippet: String,
        latest: u64,
    }
    let mut groups: BTreeMap<(String, String), Group> = BTreeMap::new();
    // Gate on the same half-open window as other delta sources: strictly after the
    // seen-cursor and no later than wall-clock `now`. The upper bound matters
    // because the seen-cursor only ever advances to `now`, so a future-dated
    // (clock-skewed-ahead) reaction would otherwise satisfy `> cursor` and
    // re-render every turn until wall-clock caught up.
    for cap in sources
        .iter()
        .filter(|c| c.created_at > cursor && c.created_at <= now)
    {
        let key = (cap.target_message_id.clone(), cap.emoji.clone());
        let group = groups.entry(key).or_insert_with(|| Group {
            reactors: Vec::new(),
            emoji: cap.emoji.clone(),
            target_snippet: cap.target_snippet.clone(),
            latest: 0,
        });
        if !group.reactors.contains(&cap.reactor_ref) {
            group.reactors.push(cap.reactor_ref.clone());
        }
        group.latest = group.latest.max(cap.created_at);
    }
    let mut ordered: Vec<Group> = groups.into_values().collect();
    ordered.sort_by(|a, b| {
        a.latest
            .cmp(&b.latest)
            .then_with(|| a.target_snippet.cmp(&b.target_snippet))
            .then_with(|| a.emoji.cmp(&b.emoji))
    });
    let total = ordered.len();
    let omitted = total.saturating_sub(MAX_REACTION_ROWS);
    let rows = ordered
        .into_iter()
        .take(MAX_REACTION_ROWS)
        .map(|mut g| {
            g.reactors.sort();
            ReactionRow {
                reactors: g.reactors,
                emoji: g.emoji,
                target_snippet: g.target_snippet,
                age: relative_time(g.latest, now),
            }
        })
        .collect();
    (rows, omitted)
}

fn snippet(body: &str) -> String {
    let trimmed = body.trim();
    let mut out: String = trimmed.chars().take(SNIPPET_CHARS).collect();
    if trimmed.chars().count() > SNIPPET_CHARS {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(reactor: &str, emoji: &str, target: &str, at: u64) -> ReactionCap {
        ReactionCap {
            reactor_ref: reactor.into(),
            emoji: emoji.into(),
            target_message_id: target.into(),
            target_snippet: "snippet".into(),
            created_at: at,
        }
    }

    #[test]
    fn groups_by_message_and_emoji_and_gates_on_cursor() {
        let sources = vec![
            cap("a", "👍", "m1", 20),
            cap("b", "👍", "m1", 21),
            cap("c", "🎉", "m1", 22),
            cap("d", "👍", "m1", 5), // before cursor → dropped
        ];
        let (rows, omitted) = group_reactions(&sources, 10, 100);
        assert_eq!(omitted, 0);
        assert_eq!(rows.len(), 2, "👍/m1 and 🎉/m1");
        let thumbs = rows.iter().find(|r| r.emoji == "👍").unwrap();
        assert_eq!(thumbs.reactors, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn cursor_past_all_reactions_yields_nothing() {
        let sources = vec![cap("a", "👍", "m1", 20)];
        let (rows, _) = group_reactions(&sources, 20, 100);
        assert!(rows.is_empty());
    }

    #[test]
    fn future_dated_reaction_is_not_rendered_until_now_catches_up() {
        // A reactor whose clock runs ahead: created_at (150) > cursor (10) but also
        // > now (100). It must NOT render this turn, or it would repeat every turn
        // until wall-clock passed it (the seen-cursor only advances to `now`).
        let sources = vec![cap("a", "👍", "m1", 150)];
        let (rows, _) = group_reactions(&sources, 10, 100);
        assert!(rows.is_empty(), "future-dated reaction must be withheld");
        // Once wall-clock passes it, it renders exactly once.
        let (rows, _) = group_reactions(&sources, 10, 200);
        assert_eq!(rows.len(), 1);
    }
}
