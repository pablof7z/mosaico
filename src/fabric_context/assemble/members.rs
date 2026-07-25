use std::collections::{BTreeMap, BTreeSet};

use super::projected_presence;
use crate::fabric_context::capture::{MembersInput, StatusCap, ViewInputs};
use crate::fabric_context::model::{MemberKind, MemberRow};
use crate::util::relative_time;

/// The `since` label for a member the fabric knows nothing live about.
const UNKNOWN: &str = "unknown";

/// Full-snapshot member rows from the frozen roster, profile, and status inputs.
///
/// A member earns a row from either a live heartbeat OR observed kind:9 activity
/// in the channel. Only a live heartbeat carries a `state` label — presence is a
/// lifecycle fact, and a peer we have merely seen talking has no lifecycle we can
/// vouch for. Two kinds of non-self member are dropped instead of rendered: one
/// that is unaddressable (see [`addressable`]) and one that is inert — no
/// heartbeat and no activity at all, so there is nothing to say about it beyond
/// its bare existence. Self always survives, so an agent never loses sight of
/// itself.
pub(super) fn member_rows(inputs: &ViewInputs, channel: &str, now: u64) -> Vec<MemberRow> {
    let members = &inputs.members;
    let statuses = inputs
        .presence
        .statuses
        .get(channel)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let status_map = live_status_map(statuses, now);

    members
        .roster
        .get(channel)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|(pk, _)| !members.backend.contains(pk))
        .filter_map(|(pk, _role)| {
            let is_self = pk == inputs.meta.self_pubkey;
            let status = status_map.get(&pk);
            let presence = status.map(|status| projected_presence(status, now));
            // A live heartbeat owns both the state label and its `since`; without
            // one, the most recent thing the member said stands in for liveness so
            // a talkative peer with no presence lease still reads as recently here.
            let (state, since) = match presence.as_ref() {
                Some(row) => (Some(row.state), relative_time(row.state_since, now)),
                None => match members.activity_at(channel, &pk) {
                    Some(at) => (None, relative_time(at, now)),
                    None => (None, UNKNOWN.to_string()),
                },
            };
            if !is_self && (!addressable(members, &pk, status) || since == UNKNOWN) {
                return None;
            }
            let status_text = presence
                .as_ref()
                .map(crate::session_presence::PublicPresence::text)
                .unwrap_or_default();
            let kind = if is_self
                || status.is_some()
                || members
                    .agent_slugs
                    .get(&pk)
                    .is_some_and(|slug| !slug.trim().is_empty())
            {
                MemberKind::Agent
            } else {
                MemberKind::Human
            };
            Some(MemberRow {
                kind,
                name: reference(inputs, &pk, status),
                state,
                status: status_text,
                since,
            })
        })
        .collect()
}

/// Whether the member has a name an agent could actually address. A kind:0
/// handle is the durable answer; a live status carries its own public slug and
/// stands in when the profile has not been fetched yet. With neither, the only
/// thing [`reference`] can produce is a truncated pubkey — a row that costs the
/// agent attention and gives it nothing to act on.
fn addressable(members: &MembersInput, pk: &str, status: Option<&&StatusCap>) -> bool {
    members.has_handle(pk) || status.is_some_and(|s| !s.slug.trim().is_empty())
}

/// Roster pubkeys with no resolvable kind:0 handle, across every captured
/// channel. The daemon turns this into a debounced profile refetch, so a roster
/// that had to withhold or improvise a name repairs itself on a later turn
/// instead of staying degraded. Self and this daemon's own backend key are never
/// included.
pub(crate) fn missing_profile_pubkeys(inputs: &ViewInputs) -> Vec<String> {
    let members = &inputs.members;
    let self_pubkey = &inputs.meta.self_pubkey;
    members
        .roster
        .values()
        .flat_map(BTreeMap::keys)
        .filter(|pk| *pk != self_pubkey && !members.backend.contains(*pk))
        .filter(|pk| !members.has_handle(pk))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Live statuses keyed by pubkey, preserving the updated_at DESC last insert.
fn live_status_map(statuses: &[StatusCap], now: u64) -> BTreeMap<String, &StatusCap> {
    statuses
        .iter()
        .filter(|s| s.expiration.is_none_or(|expiration| expiration >= now))
        .map(|s| (s.pubkey.clone(), s))
        .collect()
}

fn reference(inputs: &ViewInputs, pk: &str, status: Option<&&StatusCap>) -> String {
    if pk == inputs.meta.self_pubkey {
        return inputs.meta.self_ref.clone();
    }
    member_reference(&inputs.members, &inputs.meta.local_host, pk, status)
}

fn member_reference(
    members: &MembersInput,
    _meta_local_host: &str,
    pk: &str,
    status: Option<&&StatusCap>,
) -> String {
    if let Some(slug) = status
        .map(|s| s.slug.trim())
        .filter(|slug| !slug.is_empty())
    {
        return slug.to_string();
    }
    members.refs.get(pk).cloned().unwrap_or_default()
}
