//! Pure per-entity subscription filter builders for the daemon's single relay
//! connection.
//!
//! This module is now PURE plumbing: it computes the narrow NIP-29 [`Filter`]
//! and the semantic [`SubscriptionId`] for a single covered entity (a channel
//! `#h`, a group-state `#d`, or an addressed pubkey `#p`). It holds no state and
//! talks to no network.
//!
//! The old aggregate-REQ registry (three shared `#h`/`#p`/`#d` filters plus
//! narrow add-REQs, compacted at quiet boundaries) has been RETIRED. Shrinking a
//! shared aggregate on teardown made the relay replay every stored event for
//! every remaining entity, so teardown was never implemented and subscriptions
//! leaked without bound. Coverage is now owned by
//! [`crate::reconcile::subscriptions::SubscriptionReconciler`], which opens ONE
//! narrow REQ per entity, refcounts it across the sessions that need it, and
//! closes it — with a real NIP-01 CLOSE — when the last owner drops it. These
//! builders are the leaf the reconciler's planner calls to shape each REQ.

use crate::fabric::nip29::wire::{
    kind, KIND_AGENT_ROSTER, KIND_CHAT, KIND_GROUP_ADMINS, KIND_GROUP_MEMBERS, KIND_GROUP_METADATA,
    KIND_LONGFORM, KIND_STATUS,
};
use nostr_sdk::prelude::{Alphabet, Filter, SingleLetterTag, SubscriptionId};

// ── Semantic per-entity subscription ids ────────────────────────────────────────

/// Semantic id for a channel's narrow `#h` REQ. Stable per channel so
/// re-applying it REPLACES the relay-side REQ in place (NIP-01) — that is how
/// spawn-on-mention chat replay re-streams a channel's stored events without
/// opening a second concurrent REQ.
pub(crate) fn id_h_narrow(h: &str) -> SubscriptionId {
    SubscriptionId::new(format!("te-v2-h-{h}"))
}
/// Semantic id for an addressed pubkey's narrow `#p` REQ.
pub(crate) fn id_p_narrow(pk: &str) -> SubscriptionId {
    SubscriptionId::new(format!("te-v2-p-{pk}"))
}
/// Semantic id for a group's narrow group-state `#d` REQ.
pub(crate) fn id_gstate_narrow(h: &str) -> SubscriptionId {
    SubscriptionId::new(format!("te-v2-gstate-{h}"))
}
/// Semantic id for one daemon-lifetime, unscoped kind subscription.
pub(crate) fn id_global_kind(kind: u16) -> SubscriptionId {
    SubscriptionId::new(format!("te-v2-global-kind-{kind}"))
}

// ── Pure filter builders ────────────────────────────────────────────────────────

fn h_single() -> SingleLetterTag {
    SingleLetterTag::lowercase(Alphabet::H)
}
fn p_single() -> SingleLetterTag {
    SingleLetterTag::lowercase(Alphabet::P)
}

/// Narrow `#h` filter for a single channel: chat + status + long-form + backend
/// capability roster scoped to exactly one NIP-29 group id.
pub(crate) fn narrow_h_filter(h: &str) -> Filter {
    Filter::new()
        .kinds([
            kind(KIND_CHAT),
            kind(KIND_STATUS),
            kind(KIND_AGENT_ROSTER),
            kind(KIND_LONGFORM),
        ])
        .custom_tag(h_single(), h)
}

/// Narrow `#p` filter for a single pubkey: chat + long-form addressed to one
/// pubkey. NOT status (30315) — presence is channel-scoped, never p-addressed.
pub(crate) fn narrow_p_filter(pk: &str) -> Filter {
    Filter::new()
        .kinds([kind(KIND_CHAT), kind(KIND_LONGFORM)])
        .custom_tag(p_single(), pk)
}

/// Narrow group-state filter for a single group id: relay-authored
/// metadata/admins/members scoped by `#d` (identifier).
pub(crate) fn narrow_gstate_filter(h: &str) -> Filter {
    Filter::new()
        .kinds([
            kind(KIND_GROUP_METADATA),
            kind(KIND_GROUP_ADMINS),
            kind(KIND_GROUP_MEMBERS),
        ])
        .identifier(h)
}

/// Unscoped discovery filter. Kind:9000 exposes every pubkey entering any
/// channel; the demux then warms that pubkey's kind:0 on demand.
pub(crate) fn global_kind_filter(kind_number: u16) -> Filter {
    Filter::new().kind(kind(kind_number))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_h_filter_has_h_tag_and_chat_status_longform_kinds_not_profile() {
        let f = narrow_h_filter("room1");
        let json = serde_json::to_string(&f).unwrap();
        assert!(json.contains("\"#h\""), "must scope by #h: {json}");
        assert!(
            json.contains("room1"),
            "must scope to the one channel: {json}"
        );
        assert!(json.contains('9'), "kind 9 present");
        assert!(json.contains("30315"), "kind 30315 present");
        assert!(json.contains("30555"), "kind 30555 present");
        assert!(json.contains("30023"), "kind 30023 present");
        assert!(!json.contains("\"kinds\":[0"), "no profile kind 0: {json}");
    }

    #[test]
    fn narrow_p_filter_has_p_tag_chat_and_longform_but_not_status() {
        let f = narrow_p_filter("pk-a");
        let json = serde_json::to_string(&f).unwrap();
        assert!(json.contains("\"#p\""), "must scope by #p: {json}");
        assert!(
            json.contains("pk-a"),
            "must scope to the one pubkey: {json}"
        );
        assert!(json.contains('9'), "kind 9 present");
        assert!(json.contains("30023"), "kind 30023 present");
        assert!(
            !json.contains("30315"),
            "status is channel-scoped, never p-addressed: {json}"
        );
    }

    #[test]
    fn narrow_gstate_filter_has_d_tag_and_membership_kind() {
        let f = narrow_gstate_filter("room1");
        let json = serde_json::to_string(&f).unwrap();
        assert!(json.contains("\"#d\""), "must scope by #d: {json}");
        assert!(
            json.contains("room1"),
            "must scope to the one group: {json}"
        );
        assert!(json.contains("39002"), "members kind present: {json}");
    }

    #[test]
    fn no_narrow_filter_ever_carries_kind_zero() {
        // Profiles resolve on-demand via `Transport::fetch` + `profile.rs`; no live
        // subscription may pull the kind:0 firehose that this module replaced.
        for f in [
            narrow_h_filter("room"),
            narrow_p_filter("pk"),
            narrow_gstate_filter("room"),
        ] {
            let json = serde_json::to_string(&f).unwrap();
            assert!(
                !json.contains("\"kinds\":[0") && !json.contains(",0,") && !json.contains("[0]"),
                "no live subscription may carry kind:0 — {json}"
            );
        }
    }

    #[test]
    fn global_kind_filter_is_unscoped_put_user_only() {
        let put_user = crate::fabric::nip29::wire::KIND_GROUP_PUT_USER;
        let filter = global_kind_filter(put_user);
        let json = serde_json::to_string(&filter).unwrap();
        assert_eq!(json, r#"{"kinds":[9000]}"#);
        assert_eq!(
            id_global_kind(put_user).to_string(),
            "te-v2-global-kind-9000"
        );
    }
}
