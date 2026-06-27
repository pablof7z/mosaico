//! Pure subscription-planning data structure for the daemon's single relay
//! connection.
//!
//! This module computes WHICH relay subscriptions (REQs) should exist and
//! returns deltas (open/close lists). It does NOT talk to the network — the
//! daemon applies the returned [`PlannedReq`]s and close-ids via the transport
//! layer. Keeping the planner pure makes the whole subscription model unit
//! testable without a relay.
//!
//! ## The model
//!
//! The daemon holds ONE relay connection. Instead of per-(project×kind)
//! filters we maintain THREE STABLE aggregate REQs plus narrow add-REQs:
//!
//! - Aggregate `#h`: kinds `[9, 30315, 30023]`, filtered by `#h` (NIP-29 group
//!   id) over ALL active channels.
//! - Aggregate `#p`: kinds `[9, 30023]`, filtered by `#p` over ALL durable
//!   pubkeys we care about.
//! - Aggregate group-state: kinds `[39000, 39001, 39002]`, filtered by `#d`
//!   (identifier) over ALL group ids.
//!
//! When a NEW channel or pubkey appears at runtime we add a NARROW REQ scoped
//! to just that entity instead of mutating the aggregate (mutating an aggregate
//! makes the relay replay every stored event for every tracked entity). At a
//! quiet boundary we COMPACT: rebuild the aggregates over the union and CLOSE
//! the narrow REQs now subsumed by them.

use crate::fabric::nip29::wire::{
    kind, KIND_CHAT, KIND_GROUP_ADMINS, KIND_GROUP_MEMBERS, KIND_GROUP_METADATA, KIND_LONGFORM,
    KIND_STATUS,
};
use nostr_sdk::prelude::{Alphabet, Filter, SingleLetterTag, SubscriptionId};
use std::collections::BTreeSet;

// ── Semantic subscription ids ──────────────────────────────────────────────────

const ID_H_ALL: &str = "te-v2-h-all";
const ID_P_ALL: &str = "te-v2-p-all";
const ID_GSTATE_ALL: &str = "te-v2-gstate-all";

fn id_h_all() -> SubscriptionId {
    SubscriptionId::new(ID_H_ALL)
}
fn id_p_all() -> SubscriptionId {
    SubscriptionId::new(ID_P_ALL)
}
fn id_gstate_all() -> SubscriptionId {
    SubscriptionId::new(ID_GSTATE_ALL)
}
fn id_h_narrow(h: &str) -> SubscriptionId {
    SubscriptionId::new(format!("te-v2-h-{h}"))
}
// Part of the registry's runtime-pubkey-add path (`add_pubkey`), present for API
// completeness but not yet wired — new ordinal pubkeys currently enter #p
// coverage via the next `seed`/`build_entity_coverage` rather than a narrow add.
#[allow(dead_code)]
fn id_p_narrow(pk: &str) -> SubscriptionId {
    SubscriptionId::new(format!("te-v2-p-{pk}"))
}
fn id_gstate_narrow(h: &str) -> SubscriptionId {
    SubscriptionId::new(format!("te-v2-gstate-{h}"))
}

// ── Pure filter builders ────────────────────────────────────────────────────────

fn h_single() -> SingleLetterTag {
    SingleLetterTag::lowercase(Alphabet::H)
}
fn p_single() -> SingleLetterTag {
    SingleLetterTag::lowercase(Alphabet::P)
}

/// Aggregate `#h` filter: chat + status + long-form, scoped to `channels`.
pub(crate) fn aggregate_h_filter(channels: &BTreeSet<String>) -> Filter {
    let f = Filter::new().kinds([kind(KIND_CHAT), kind(KIND_STATUS), kind(KIND_LONGFORM)]);
    if channels.is_empty() {
        f
    } else {
        f.custom_tags(h_single(), channels.iter().cloned())
    }
}

/// Aggregate `#p` filter: chat + long-form addressed to `pubkeys`. NOT status
/// (30315) — presence is channel-scoped, never p-addressed.
pub(crate) fn aggregate_p_filter(pubkeys: &BTreeSet<String>) -> Filter {
    let f = Filter::new().kinds([kind(KIND_CHAT), kind(KIND_LONGFORM)]);
    if pubkeys.is_empty() {
        f
    } else {
        f.custom_tags(p_single(), pubkeys.iter().cloned())
    }
}

/// Aggregate group-state filter: relay-authored metadata/admins/members,
/// scoped by `#d` (group id == identifier) over `groups`.
pub(crate) fn aggregate_gstate_filter(groups: &BTreeSet<String>) -> Filter {
    let f = Filter::new().kinds([
        kind(KIND_GROUP_METADATA),
        kind(KIND_GROUP_ADMINS),
        kind(KIND_GROUP_MEMBERS),
    ]);
    if groups.is_empty() {
        f
    } else {
        f.identifiers(groups.iter().cloned())
    }
}

/// Narrow `#h` filter for a single channel.
pub(crate) fn narrow_h_filter(h: &str) -> Filter {
    Filter::new()
        .kinds([kind(KIND_CHAT), kind(KIND_STATUS), kind(KIND_LONGFORM)])
        .custom_tag(h_single(), h)
}

/// A [`PlannedReq`] that, when applied, REPLAYS channel `h`'s stored chat. It
/// reuses the channel's narrow `#h` subscription id, so re-applying it replaces
/// that REQ in place (NIP-01) and the relay re-streams matching stored events
/// rather than opening an extra concurrent REQ. Used when a session becomes
/// alive AFTER a message was published to its channel (the spawn-on-mention
/// case): the live materialize path only routes to sessions already alive, so
/// without a replay the triggering message would never reach the new session.
pub(crate) fn channel_chat_replay_req(h: &str) -> PlannedReq {
    PlannedReq {
        id: id_h_narrow(h),
        filter: narrow_h_filter(h),
    }
}

/// Narrow `#p` filter for a single pubkey. Used by the not-yet-wired
/// `add_pubkey` runtime path; see `id_p_narrow`.
#[allow(dead_code)]
pub(crate) fn narrow_p_filter(pk: &str) -> Filter {
    Filter::new()
        .kinds([kind(KIND_CHAT), kind(KIND_LONGFORM)])
        .custom_tag(p_single(), pk)
}

/// Narrow group-state filter for a single group id.
pub(crate) fn narrow_gstate_filter(h: &str) -> Filter {
    Filter::new()
        .kinds([
            kind(KIND_GROUP_METADATA),
            kind(KIND_GROUP_ADMINS),
            kind(KIND_GROUP_MEMBERS),
        ])
        .identifier(h)
}

// ── Coverage + planned-REQ types ────────────────────────────────────────────────

/// The set of entities the daemon currently wants live coverage for.
#[derive(Default, Clone, PartialEq, Eq, Debug)]
pub(crate) struct EntityCoverage {
    /// NIP-29 group ids (h tags).
    pub channels_h: BTreeSet<String>,
    /// Durable pubkeys (p tags).
    pub addressed_pubkeys_p: BTreeSet<String>,
    /// Group ids for 39000/39001/39002 (d tags).
    pub group_state_d: BTreeSet<String>,
}

/// One live REQ the registry is tracking.
#[derive(Clone, Debug)]
pub(crate) struct PlannedReq {
    pub id: SubscriptionId,
    pub filter: Filter,
}

/// Plans the three aggregate REQs and any narrow add-REQs. Pure: holds only the
/// coverage bookkeeping, never a transport handle.
pub(crate) struct SubscriptionRegistry {
    aggregate: EntityCoverage,
    narrow: EntityCoverage,
}

impl SubscriptionRegistry {
    pub fn new() -> Self {
        Self {
            aggregate: EntityCoverage::default(),
            narrow: EntityCoverage::default(),
        }
    }

    /// Plan the three aggregate REQs for the given coverage. Returns the REQs to
    /// open. Sets `aggregate = coverage` and clears `narrow`.
    pub fn seed(&mut self, coverage: EntityCoverage) -> Vec<PlannedReq> {
        self.aggregate = coverage;
        self.narrow = EntityCoverage::default();
        self.aggregate_reqs()
    }

    /// Whether channel `h` already has live `#h` coverage (aggregate or narrow).
    /// Used to decide if a newly-alive session needs a chat replay: if the
    /// channel was ALREADY covered, messages may have arrived before this session
    /// existed (spawn-on-mention) and must be replayed; if it is about to be
    /// freshly subscribed, the relay streams its backlog to this session anyway.
    pub fn covers_channel(&self, h: &str) -> bool {
        self.aggregate.channels_h.contains(h) || self.narrow.channels_h.contains(h)
    }

    /// Register a new channel. Returns the narrow REQs to open (one `#h`, one
    /// group-state `#d`) — empty if already covered by aggregate or narrow.
    pub fn add_channel(&mut self, h: &str) -> Vec<PlannedReq> {
        if self.aggregate.channels_h.contains(h) || self.narrow.channels_h.contains(h) {
            return Vec::new();
        }
        self.narrow.channels_h.insert(h.to_string());
        self.narrow.group_state_d.insert(h.to_string());
        vec![
            PlannedReq {
                id: id_h_narrow(h),
                filter: narrow_h_filter(h),
            },
            PlannedReq {
                id: id_gstate_narrow(h),
                filter: narrow_gstate_filter(h),
            },
        ]
    }

    /// Register a new pubkey. Returns the narrow `#p` REQ to open — empty if
    /// already covered by aggregate or narrow. Not yet wired into the daemon;
    /// new ordinal pubkeys currently enter `#p` via the next aggregate seed.
    #[allow(dead_code)]
    pub fn add_pubkey(&mut self, pubkey: &str) -> Vec<PlannedReq> {
        if self.aggregate.addressed_pubkeys_p.contains(pubkey)
            || self.narrow.addressed_pubkeys_p.contains(pubkey)
        {
            return Vec::new();
        }
        self.narrow.addressed_pubkeys_p.insert(pubkey.to_string());
        vec![PlannedReq {
            id: id_p_narrow(pubkey),
            filter: narrow_p_filter(pubkey),
        }]
    }

    /// Fold all narrow entities into the aggregate. Returns `(to_open, to_close)`:
    /// `to_open` = the three rebuilt aggregate REQs covering the union;
    /// `to_close` = the narrow ids now subsumed. Clears `narrow`. Not yet wired
    /// into the daemon — compaction at a quiet boundary is a future optimization.
    #[allow(dead_code)]
    pub fn compact(&mut self) -> (Vec<PlannedReq>, Vec<SubscriptionId>) {
        let mut to_close = Vec::new();
        for h in &self.narrow.channels_h {
            to_close.push(id_h_narrow(h));
        }
        for h in &self.narrow.group_state_d {
            to_close.push(id_gstate_narrow(h));
        }
        for pk in &self.narrow.addressed_pubkeys_p {
            to_close.push(id_p_narrow(pk));
        }

        // Fold narrow → aggregate, then clear narrow.
        self.aggregate
            .channels_h
            .extend(std::mem::take(&mut self.narrow.channels_h));
        self.aggregate
            .addressed_pubkeys_p
            .extend(std::mem::take(&mut self.narrow.addressed_pubkeys_p));
        self.aggregate
            .group_state_d
            .extend(std::mem::take(&mut self.narrow.group_state_d));

        (self.aggregate_reqs(), to_close)
    }

    /// Build the three stable aggregate REQs from the current aggregate coverage.
    fn aggregate_reqs(&self) -> Vec<PlannedReq> {
        vec![
            PlannedReq {
                id: id_h_all(),
                filter: aggregate_h_filter(&self.aggregate.channels_h),
            },
            PlannedReq {
                id: id_p_all(),
                filter: aggregate_p_filter(&self.aggregate.addressed_pubkeys_p),
            },
            PlannedReq {
                id: id_gstate_all(),
                filter: aggregate_gstate_filter(&self.aggregate.group_state_d),
            },
        ]
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn set<const N: usize>(items: [&str; N]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn ids(reqs: &[PlannedReq]) -> Vec<String> {
        reqs.iter().map(|r| r.id.to_string()).collect()
    }

    #[test]
    fn seed_returns_three_aggregate_reqs_with_semantic_ids() {
        let channels: BTreeSet<String> = (0..14).map(|i| format!("chan-{i}")).collect();
        let coverage = EntityCoverage {
            channels_h: channels.clone(),
            addressed_pubkeys_p: set(["pk-a", "pk-b"]),
            group_state_d: channels,
        };
        let mut reg = SubscriptionRegistry::new();
        let reqs = reg.seed(coverage);
        assert_eq!(reqs.len(), 3, "seed always plans exactly three aggregates");
        assert_eq!(ids(&reqs), vec![ID_H_ALL, ID_P_ALL, ID_GSTATE_ALL]);
    }

    #[test]
    fn seed_produces_exactly_three_reqs_and_never_kind_zero() {
        // Wiring contract for the daemon: one seed → three aggregate REQs, and not
        // one of them may carry kind:0 (profiles resolve on-demand, never via a
        // live subscription — that firehose is what this whole module replaced).
        let channels = set(["room-a", "room-b"]);
        let coverage = EntityCoverage {
            channels_h: channels.clone(),
            addressed_pubkeys_p: set(["pk-1", "pk-2", "pk-3"]),
            group_state_d: channels,
        };
        let mut reg = SubscriptionRegistry::new();
        let reqs = reg.seed(coverage);

        assert_eq!(reqs.len(), 3, "seed plans exactly three aggregate REQs");
        for req in &reqs {
            let json = serde_json::to_string(&req.filter).unwrap();
            assert!(
                !json.contains("\"kinds\":[0") && !json.contains(",0,") && !json.contains("[0]"),
                "no live subscription may carry kind:0 — {}: {json}",
                req.id
            );
        }
    }

    #[test]
    fn aggregate_h_filter_has_h_tag_and_chat_status_longform_kinds_not_profile() {
        let f = aggregate_h_filter(&set(["room1", "room2"]));
        let json = serde_json::to_string(&f).unwrap();
        assert!(json.contains("\"#h\""), "must scope by #h: {json}");
        assert!(json.contains('9'), "kind 9 present");
        assert!(json.contains("30315"), "kind 30315 present");
        assert!(json.contains("30023"), "kind 30023 present");
        assert!(!json.contains("\"kinds\":[0"), "no profile kind 0: {json}");
    }

    #[test]
    fn aggregate_p_filter_has_p_tag_chat_and_longform_but_not_status() {
        let f = aggregate_p_filter(&set(["pk-a", "pk-b"]));
        let json = serde_json::to_string(&f).unwrap();
        assert!(json.contains("\"#p\""), "must scope by #p: {json}");
        assert!(json.contains('9'), "kind 9 present");
        assert!(json.contains("30023"), "kind 30023 present");
        assert!(
            !json.contains("30315"),
            "status is channel-scoped, never p-addressed: {json}"
        );
    }

    #[test]
    fn gstate_filter_has_d_tag_and_membership_kind() {
        let f = aggregate_gstate_filter(&set(["room1"]));
        let json = serde_json::to_string(&f).unwrap();
        assert!(json.contains("\"#d\""), "must scope by #d: {json}");
        assert!(json.contains("39002"), "members kind present: {json}");
    }

    #[test]
    fn add_channel_is_idempotent() {
        let mut reg = SubscriptionRegistry::new();
        reg.seed(EntityCoverage {
            channels_h: set(["existing"]),
            addressed_pubkeys_p: BTreeSet::new(),
            group_state_d: set(["existing"]),
        });

        let opened = reg.add_channel("newroom");
        assert_eq!(opened.len(), 2, "one #h narrow + one gstate narrow");
        assert_eq!(
            ids(&opened),
            vec!["te-v2-h-newroom", "te-v2-gstate-newroom"]
        );

        let again = reg.add_channel("newroom");
        assert!(again.is_empty(), "second add is a no-op");

        let covered = reg.add_channel("existing");
        assert!(covered.is_empty(), "aggregate-covered channel is a no-op");
    }

    #[test]
    fn add_pubkey_is_idempotent() {
        let mut reg = SubscriptionRegistry::new();
        reg.seed(EntityCoverage::default());

        let opened = reg.add_pubkey("pk-new");
        assert_eq!(opened.len(), 1);
        assert_eq!(ids(&opened), vec!["te-v2-p-pk-new"]);

        let again = reg.add_pubkey("pk-new");
        assert!(again.is_empty(), "second add is a no-op");
    }

    #[test]
    fn compact_folds_narrows_into_aggregate_and_closes_them() {
        let mut reg = SubscriptionRegistry::new();
        reg.seed(EntityCoverage {
            channels_h: set(["seedroom"]),
            addressed_pubkeys_p: set(["seedpk"]),
            group_state_d: set(["seedroom"]),
        });

        reg.add_channel("newroom");
        reg.add_pubkey("newpk");

        let (to_open, to_close) = reg.compact();

        assert_eq!(to_open.len(), 3, "three rebuilt aggregates");
        assert_eq!(ids(&to_open), vec![ID_H_ALL, ID_P_ALL, ID_GSTATE_ALL]);

        // The rebuilt aggregates cover the union (seed + narrows).
        let h_json = serde_json::to_string(&to_open[0].filter).unwrap();
        assert!(h_json.contains("seedroom") && h_json.contains("newroom"));
        let p_json = serde_json::to_string(&to_open[1].filter).unwrap();
        assert!(p_json.contains("seedpk") && p_json.contains("newpk"));
        let g_json = serde_json::to_string(&to_open[2].filter).unwrap();
        assert!(g_json.contains("seedroom") && g_json.contains("newroom"));

        // The narrow ids that are now subsumed must be closed.
        let close_ids: BTreeSet<String> = to_close.iter().map(|i| i.to_string()).collect();
        assert!(close_ids.contains("te-v2-h-newroom"));
        assert!(close_ids.contains("te-v2-gstate-newroom"));
        assert!(close_ids.contains("te-v2-p-newpk"));

        // Narrow is cleared: re-adding now returns empty (covered by aggregate).
        assert!(reg.add_channel("newroom").is_empty());
        assert!(reg.add_pubkey("newpk").is_empty());
    }
}
