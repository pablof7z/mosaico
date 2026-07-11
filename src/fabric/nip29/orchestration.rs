//! NIP-29 "orchestration" event: a `kind:9` group-chat event that asks backends
//! to add agent roles into a freshly-created child group.
//!
//! The orchestration event is routed into the COORDINATION group (its routing `h`
//! equals the parent group id) so every backend already in the parent sees it. A
//! `kind:9` has a single routing `h`, so the child group id travels separately in
//! an `h-target` tag. All semantics live in the structured tags; the prose content
//! is advisory and is IGNORED by receivers — a plain `kind:9` with no `te-op` tag
//! is just chat and parses to `None`.

use crate::fabric::nip29::wire::{kind, KIND_CHAT};
use anyhow::Result;
use nostr_sdk::prelude::*;
use std::collections::HashMap;

/// Marker `te-op` value identifying the add-agents orchestration event.
pub const TE_OP_ADD_AGENTS: &str = "subgroup.add-agents.v2";

fn tag(parts: &[&str]) -> Result<Tag> {
    Ok(Tag::parse(parts.iter().copied())?)
}

/// One backend-targeted add: route agent `slug` to backend `backend_pubkey`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddTarget {
    pub backend_pubkey: String,
    /// Agent identity slug (the `~/.tenex-edge/agents/*.json` filename stem).
    pub slug: String,
    /// Optional permanent session pubkey to resume instead of spawning fresh.
    pub session_pubkey: Option<String>,
}

/// Build the `kind:9` add-agents orchestration event.
///
/// Routed into the coordination group (`h == parent_h`) so backends already in the
/// parent group receive it. The child group id travels in an `h-target` tag.
///
/// Tags, in order:
///   `["h", parent_h]`, `["te-op", TE_OP_ADD_AGENTS]`, `["parent", parent_h]`,
///   `["h-target", child_h]`, one `["p", backend_pubkey]` per DISTINCT backend
///   (deduped, sorted for stable order), then one `["add", backend_pubkey,
///   slug, optional-session-pubkey]` per entry in `adds` (input order preserved).
///   Content = `prose`.
pub fn build_add_agents_event(
    parent_h: &str,
    child_h: &str,
    adds: &[AddTarget],
    prose: &str,
) -> Result<EventBuilder> {
    let mut tags: Vec<Tag> = vec![
        tag(&["h", parent_h])?,
        tag(&["te-op", TE_OP_ADD_AGENTS])?,
        tag(&["parent", parent_h])?,
        tag(&["h-target", child_h])?,
    ];

    // Distinct backend pubkeys, sorted for a stable wire order.
    let mut backends: Vec<&str> = adds.iter().map(|a| a.backend_pubkey.as_str()).collect();
    backends.sort_unstable();
    backends.dedup();
    for pk in backends {
        tags.push(tag(&["p", pk])?);
    }

    // One add tag per entry, preserving caller order.
    for a in adds {
        if let Some(session_pubkey) = a.session_pubkey.as_deref().filter(|s| !s.is_empty()) {
            tags.push(tag(&["add", &a.backend_pubkey, &a.slug, session_pubkey])?);
        } else {
            tags.push(tag(&["add", &a.backend_pubkey, &a.slug])?);
        }
    }

    Ok(EventBuilder::new(kind(KIND_CHAT), prose)
        .tags(tags)
        .allow_self_tagging())
}

/// A parsed add-agents orchestration event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddAgentsOp {
    pub parent: String,
    /// Child group id, from the `h-target` tag.
    pub child_h: String,
    pub adds: Vec<AddTarget>,
}

/// Parse an event as an add-agents orchestration event, reading ONLY the
/// structured tags (the prose content is ignored).
///
/// Returns `Some` only if the event is well-formed:
///
/// - `kind == 9`
/// - has `["te-op", TE_OP_ADD_AGENTS]` (this is what makes a prose-only kind:9
///   ignored)
/// - the single routing `["h", _]` equals the single `["parent", _]`
/// - exactly one `["h-target", _]`
/// - at least one `["add", pubkey, slug]` (both fields present)
///
/// Otherwise returns `None`.
pub fn parse_orchestration(event: &Event) -> Option<AddAgentsOp> {
    if event.kind.as_u16() != KIND_CHAT {
        return None;
    }

    let mut te_op: Option<&str> = None;
    let mut h_vals: Vec<&str> = Vec::new();
    let mut parent_vals: Vec<&str> = Vec::new();
    let mut target_vals: Vec<&str> = Vec::new();
    let mut adds: Vec<AddTarget> = Vec::new();

    for t in event.tags.iter() {
        let s = t.as_slice();
        match s.first().map(String::as_str) {
            Some("te-op") => te_op = s.get(1).map(String::as_str),
            Some("h") => {
                if let Some(v) = s.get(1) {
                    h_vals.push(v.as_str());
                }
            }
            Some("parent") => {
                if let Some(v) = s.get(1) {
                    parent_vals.push(v.as_str());
                }
            }
            Some("h-target") => {
                if let Some(v) = s.get(1) {
                    target_vals.push(v.as_str());
                }
            }
            Some("add") => {
                if let (Some(pk), Some(slug)) = (s.get(1), s.get(2)) {
                    adds.push(AddTarget {
                        backend_pubkey: pk.clone(),
                        slug: slug.clone(),
                        session_pubkey: s.get(3).cloned().filter(|s| !s.is_empty()),
                    });
                }
            }
            _ => {}
        }
    }

    // Must be tagged as an add-agents orchestration event.
    if te_op != Some(TE_OP_ADD_AGENTS) {
        return None;
    }
    // Exactly one routing h and one parent, and they must match.
    let (&h, &parent) = match (h_vals.as_slice(), parent_vals.as_slice()) {
        ([h], [parent]) => (h, parent),
        _ => return None,
    };
    if h != parent {
        return None;
    }
    // Exactly one child target.
    let child_h = match target_vals.as_slice() {
        [child] => child.to_string(),
        _ => return None,
    };
    // At least one add.
    if adds.is_empty() {
        return None;
    }

    Some(AddAgentsOp {
        parent: parent.to_string(),
        child_h,
        adds,
    })
}

/// A signer is authorized to issue orchestration iff their role is `admin`.
pub fn is_authorized(roles: &HashMap<String, String>, signer: &str) -> bool {
    roles.get(signer).map(String::as_str) == Some("admin")
}

/// Filter the adds down to those targeting `backend_pubkey` (this backend).
pub fn adds_for_backend<'a>(adds: &'a [AddTarget], backend_pubkey: &str) -> Vec<&'a AddTarget> {
    adds.iter()
        .filter(|a| a.backend_pubkey == backend_pubkey)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(pk: &str, slug: &str) -> AddTarget {
        AddTarget {
            backend_pubkey: pk.to_string(),
            slug: slug.to_string(),
            session_pubkey: None,
        }
    }

    fn resume(pk: &str, slug: &str, session_pubkey: &str) -> AddTarget {
        AddTarget {
            backend_pubkey: pk.to_string(),
            slug: slug.to_string(),
            session_pubkey: Some(session_pubkey.to_string()),
        }
    }

    fn sign(b: EventBuilder) -> Event {
        b.sign_with_keys(&Keys::generate()).unwrap()
    }

    fn tag_count(ev: &Event, name: &str) -> usize {
        ev.tags
            .iter()
            .filter(|t| t.as_slice().first().map(String::as_str) == Some(name))
            .count()
    }

    #[test]
    fn build_parse_round_trip_preserves_order() {
        let adds = vec![
            at("bk1", "architect"),
            at("bk2", "engineer"),
            at("bk1", "qa"),
        ];
        let b = build_add_agents_event("parent-g", "child-g", &adds, "please add these").unwrap();
        let ev = sign(b);
        assert_eq!(ev.kind.as_u16(), KIND_CHAT);

        let op = parse_orchestration(&ev).expect("well-formed");
        assert_eq!(op.parent, "parent-g");
        assert_eq!(op.child_h, "child-g");
        assert_eq!(op.adds, adds, "add order preserved");
    }

    #[test]
    fn build_dedups_p_tags_but_keeps_all_adds() {
        let adds = vec![at("bk1", "architect"), at("bk1", "qa")];
        let ev = sign(build_add_agents_event("p", "c", &adds, "x").unwrap());
        // Two adds to the same backend → one p tag, two add tags.
        assert_eq!(tag_count(&ev, "p"), 1);
        assert_eq!(tag_count(&ev, "add"), 2);
    }

    #[test]
    fn build_routes_h_to_parent_and_carries_child_in_h_target() {
        let ev = sign(build_add_agents_event("p", "c", &[at("bk", "r")], "x").unwrap());
        // Single routing h equals parent; child travels in h-target.
        assert_eq!(tag_count(&ev, "h"), 1);
        assert_eq!(tag_count(&ev, "h-target"), 1);
        let op = parse_orchestration(&ev).unwrap();
        assert_eq!(op.parent, "p");
        assert_eq!(op.child_h, "c");
    }

    #[test]
    fn build_parse_preserves_optional_session_pubkey() {
        let adds = vec![resume("bk1", "architect", &"11".repeat(32))];
        let ev = sign(build_add_agents_event("p", "c", &adds, "x").unwrap());
        let op = parse_orchestration(&ev).unwrap();
        assert_eq!(op.adds, adds);
    }

    #[test]
    fn parse_none_for_plain_chat_without_te_op() {
        // A prose-only kind:9 chat message must be ignored.
        let ev = sign(
            EventBuilder::new(kind(KIND_CHAT), "just chatting")
                .tags([tag(&["h", "p"]).unwrap()])
                .allow_self_tagging(),
        );
        assert!(parse_orchestration(&ev).is_none());
    }

    #[test]
    fn parse_none_for_different_te_op() {
        let ev = sign(
            EventBuilder::new(kind(KIND_CHAT), "x")
                .tags([
                    tag(&["h", "p"]).unwrap(),
                    tag(&["te-op", "subgroup.remove-agents.v1"]).unwrap(),
                    tag(&["parent", "p"]).unwrap(),
                    tag(&["h-target", "c"]).unwrap(),
                    tag(&["add", "bk", "r"]).unwrap(),
                ])
                .allow_self_tagging(),
        );
        assert!(parse_orchestration(&ev).is_none());
    }

    #[test]
    fn parse_none_when_h_differs_from_parent() {
        let ev = sign(
            EventBuilder::new(kind(KIND_CHAT), "x")
                .tags([
                    tag(&["h", "other-group"]).unwrap(),
                    tag(&["te-op", TE_OP_ADD_AGENTS]).unwrap(),
                    tag(&["parent", "p"]).unwrap(),
                    tag(&["h-target", "c"]).unwrap(),
                    tag(&["add", "bk", "r"]).unwrap(),
                ])
                .allow_self_tagging(),
        );
        assert!(parse_orchestration(&ev).is_none());
    }

    #[test]
    fn parse_none_when_h_target_missing() {
        let ev = sign(
            EventBuilder::new(kind(KIND_CHAT), "x")
                .tags([
                    tag(&["h", "p"]).unwrap(),
                    tag(&["te-op", TE_OP_ADD_AGENTS]).unwrap(),
                    tag(&["parent", "p"]).unwrap(),
                    tag(&["add", "bk", "r"]).unwrap(),
                ])
                .allow_self_tagging(),
        );
        assert!(parse_orchestration(&ev).is_none());
    }

    #[test]
    fn parse_none_when_two_h_target_tags() {
        let ev = sign(
            EventBuilder::new(kind(KIND_CHAT), "x")
                .tags([
                    tag(&["h", "p"]).unwrap(),
                    tag(&["te-op", TE_OP_ADD_AGENTS]).unwrap(),
                    tag(&["parent", "p"]).unwrap(),
                    tag(&["h-target", "c1"]).unwrap(),
                    tag(&["h-target", "c2"]).unwrap(),
                    tag(&["add", "bk", "r"]).unwrap(),
                ])
                .allow_self_tagging(),
        );
        assert!(parse_orchestration(&ev).is_none());
    }

    #[test]
    fn parse_none_when_no_add_tags() {
        let ev = sign(
            EventBuilder::new(kind(KIND_CHAT), "x")
                .tags([
                    tag(&["h", "p"]).unwrap(),
                    tag(&["te-op", TE_OP_ADD_AGENTS]).unwrap(),
                    tag(&["parent", "p"]).unwrap(),
                    tag(&["h-target", "c"]).unwrap(),
                ])
                .allow_self_tagging(),
        );
        assert!(parse_orchestration(&ev).is_none());
    }

    #[test]
    fn parse_none_for_wrong_kind() {
        // A kind:1 with otherwise-valid tags is not an orchestration event.
        let ev = sign(
            EventBuilder::new(kind(1), "x")
                .tags([
                    tag(&["h", "p"]).unwrap(),
                    tag(&["te-op", TE_OP_ADD_AGENTS]).unwrap(),
                    tag(&["parent", "p"]).unwrap(),
                    tag(&["h-target", "c"]).unwrap(),
                    tag(&["add", "bk", "r"]).unwrap(),
                ])
                .allow_self_tagging(),
        );
        assert!(parse_orchestration(&ev).is_none());
    }

    #[test]
    fn is_authorized_only_for_admin() {
        let mut roles = HashMap::new();
        roles.insert("admin-pk".to_string(), "admin".to_string());
        roles.insert("member-pk".to_string(), "member".to_string());
        assert!(is_authorized(&roles, "admin-pk"));
        assert!(!is_authorized(&roles, "member-pk"));
        assert!(!is_authorized(&roles, "absent-pk"));
    }

    #[test]
    fn adds_for_backend_filters() {
        let adds = vec![
            at("bk1", "architect"),
            at("bk2", "engineer"),
            at("bk1", "qa"),
        ];
        let mine = adds_for_backend(&adds, "bk1");
        assert_eq!(mine.len(), 2);
        assert_eq!(mine[0].slug, "architect");
        assert_eq!(mine[1].slug, "qa");

        assert!(adds_for_backend(&adds, "bk-none").is_empty());
    }
}
