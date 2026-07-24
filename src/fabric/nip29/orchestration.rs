//! NIP-29 "orchestration" event: a `kind:9` group-chat event that asks backends
//! to add agent roles into a freshly-created child group.
//!
//! The orchestration event is routed into the COORDINATION group (its routing `h`
//! equals the parent group id) so every backend already in the parent sees it. A
//! `kind:9` has a single routing `h`, so the child group id travels separately in
//! an `h-target` tag. All semantics live in the structured tags; the prose content
//! is advisory and is IGNORED by receivers — a plain `kind:9` with no `mosaico-op` tag
//! is just chat and parses to `None`.

use crate::fabric::nip29::wire::{kind, KIND_CHAT};
use anyhow::Result;
use nostr::*;
use std::collections::HashMap;

/// Marker `mosaico-op` value identifying the add-agents orchestration event.
pub const MOSAICO_OP_ADD_AGENTS: &str = "subgroup.add-agents.v2";
/// Marker for passively admitting exact sessions that are still running.
pub const MOSAICO_OP_ADMIT_RUNNING: &str = "subgroup.admit-running.v1";

fn tag(parts: &[&str]) -> Result<Tag> {
    Ok(Tag::parse(parts.iter().copied())?)
}

/// One backend-targeted add: route agent `slug` to backend `backend_pubkey`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddTarget {
    pub backend_pubkey: String,
    /// Agent identity slug (the `~/.mosaico/agents/*.json` filename stem).
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
///   `["h", parent_h]`, `["mosaico-op", MOSAICO_OP_ADD_AGENTS]`, `["parent", parent_h]`,
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
    build_event(MOSAICO_OP_ADD_AGENTS, parent_h, child_h, adds, prose)
}

/// Build a distinct operation that only admits exact sessions that remain live.
/// Older daemons ignore this operation instead of treating it as a resume.
pub fn build_admit_running_event(
    parent_h: &str,
    child_h: &str,
    adds: &[AddTarget],
    prose: &str,
) -> Result<EventBuilder> {
    anyhow::ensure!(
        adds.iter().all(|target| target
            .session_pubkey
            .as_deref()
            .is_some_and(|pubkey| !pubkey.is_empty())),
        "admit-running orchestration requires exact session pubkeys"
    );
    build_event(MOSAICO_OP_ADMIT_RUNNING, parent_h, child_h, adds, prose)
}

fn build_event(
    operation: &str,
    parent_h: &str,
    child_h: &str,
    adds: &[AddTarget],
    prose: &str,
) -> Result<EventBuilder> {
    let mut tags: Vec<Tag> = vec![
        tag(&["h", parent_h])?,
        tag(&["mosaico-op", operation])?,
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
    /// True only for the separate admit-running operation, which never spawns
    /// or resumes a missing session.
    pub running_only: bool,
}

/// Parse an add-agents or admit-running orchestration event, reading ONLY the
/// structured tags (the prose content is ignored).
///
/// Returns `Some` only if the event is well-formed:
///
/// - `kind == 9`
/// - has one recognized `mosaico-op` (plain prose-only kind:9 stays ignored)
/// - the single routing `["h", _]` equals the single `["parent", _]`
/// - exactly one `["h-target", _]`
/// - at least one `["add", pubkey, slug]` (both fields present)
///
/// Otherwise returns `None`.
pub fn parse_orchestration(event: &Event) -> Option<AddAgentsOp> {
    if event.kind.as_u16() != KIND_CHAT {
        return None;
    }

    let mut mosaico_op: Option<&str> = None;
    let mut h_vals: Vec<&str> = Vec::new();
    let mut parent_vals: Vec<&str> = Vec::new();
    let mut target_vals: Vec<&str> = Vec::new();
    let mut adds: Vec<AddTarget> = Vec::new();

    for t in event.tags.iter() {
        let s = t.as_slice();
        match s.first().map(String::as_str) {
            Some("mosaico-op") => mosaico_op = s.get(1).map(String::as_str),
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

    let running_only = match mosaico_op {
        Some(MOSAICO_OP_ADD_AGENTS) => false,
        Some(MOSAICO_OP_ADMIT_RUNNING) => true,
        _ => return None,
    };
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
    if running_only && adds.iter().any(|target| target.session_pubkey.is_none()) {
        return None;
    }

    Some(AddAgentsOp {
        parent: parent.to_string(),
        child_h,
        adds,
        running_only,
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
#[path = "orchestration/tests.rs"]
mod tests;
