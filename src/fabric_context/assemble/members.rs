use std::collections::BTreeMap;

use super::projected_presence;
use crate::fabric_context::capture::{MembersInput, StatusCap, ViewInputs};
use crate::fabric_context::model::{MemberKind, MemberRow};
use crate::util::relative_time;

/// Full-snapshot member rows from the frozen roster, profile, and status inputs.
///
/// Every nameable roster member earns a row. Only a live heartbeat carries a
/// `state` label — presence is a
/// lifecycle fact, and a peer we have merely seen talking has no lifecycle we
/// can vouch for. Nameable roster members remain visible even without activity;
/// their row simply omits state, status, and since. Only an unaddressable member
/// is withheld until a current profile row gives it a usable name.
pub(super) fn member_rows(inputs: &ViewInputs, channel: &str, now: u64) -> Vec<MemberRow> {
    inputs
        .members
        .roster
        .get(channel)
        .cloned()
        .unwrap_or_default()
        .into_keys()
        .filter_map(|pubkey| member_row(inputs, channel, &pubkey, now))
        .collect()
}

pub(in crate::fabric_context) fn member_row(
    inputs: &ViewInputs,
    channel: &str,
    pubkey: &str,
    now: u64,
) -> Option<MemberRow> {
    let members = &inputs.members;
    if members.backend.contains(pubkey)
        || !members
            .roster
            .get(channel)
            .is_some_and(|roster| roster.contains_key(pubkey))
    {
        return None;
    }
    let statuses = inputs
        .presence
        .statuses
        .get(channel)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let status_map = status_map(statuses);
    let is_self = pubkey == inputs.meta.self_pubkey;
    let status = status_map.get(pubkey);
    let presence = status.map(|status| projected_presence(status));
    // A live heartbeat owns both the state label and its `since`; without one,
    // the most recent thing the member said stands in for liveness.
    let (state, since) = match presence.as_ref() {
        Some(row) => (Some(row.state), relative_time(row.state_since, now)),
        None => match members.activity_at(channel, pubkey) {
            Some(at) => (None, relative_time(at, now)),
            None => (None, String::new()),
        },
    };
    if !is_self && !addressable(members, pubkey, status) {
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
            .get(pubkey)
            .is_some_and(|slug| !slug.trim().is_empty())
    {
        MemberKind::Agent
    } else {
        MemberKind::Human
    };
    let (name, host, workspace, branch) = member_origin(inputs, pubkey, status, kind);
    Some(MemberRow {
        kind,
        name,
        host,
        workspace,
        branch,
        state,
        status: status_text,
        since,
    })
}

fn member_origin(
    inputs: &ViewInputs,
    pubkey: &str,
    status: Option<&&StatusCap>,
    kind: MemberKind,
) -> (String, String, String, String) {
    let mut name = reference(inputs, pubkey, status);
    if kind != MemberKind::Agent {
        return (name, String::new(), String::new(), String::new());
    }
    let host = status
        .map(|row| row.host.as_str())
        .filter(|host| !host.is_empty())
        .or_else(|| {
            inputs
                .members
                .hosts
                .get(pubkey)
                .map(String::as_str)
                .filter(|host| !host.is_empty())
        })
        .unwrap_or_default();
    let workspace = status.map(|row| row.workspace.as_str()).unwrap_or_default();
    let self_workspace = inputs
        .meta
        .self_row
        .as_ref()
        .map(|row| row.workspace.as_str())
        .unwrap_or(inputs.meta.current_workspace.trim());
    let cross_workspace = !workspace.is_empty() && workspace != self_workspace;
    let cross_host = !host.is_empty() && host != inputs.meta.local_host.trim();
    let branch = status.map(|row| row.branch.clone()).unwrap_or_default();
    if !cross_workspace && !cross_host {
        return (name, String::new(), String::new(), branch);
    }
    if !host.is_empty() {
        let suffix = format!("@{host}");
        if let Some(bare) = name.strip_suffix(&suffix) {
            name = bare.to_string();
        }
    }
    (
        name,
        host.to_string(),
        if cross_workspace {
            workspace.to_string()
        } else {
            String::new()
        },
        branch,
    )
}

/// Whether the member has a name an agent could actually address. A kind:0
/// handle is the durable answer; a current status carries its own public slug
/// and stands in when no profile row is currently available. With neither, the only
/// thing [`reference`] can produce is a truncated pubkey — a row that costs the
/// agent attention and gives it nothing to act on.
fn addressable(members: &MembersInput, pk: &str, status: Option<&&StatusCap>) -> bool {
    members.has_handle(pk) || status.is_some_and(|s| !s.slug.trim().is_empty())
}

/// NMP-current statuses keyed by pubkey, preserving delivery order.
fn status_map(statuses: &[StatusCap]) -> BTreeMap<String, &StatusCap> {
    statuses
        .iter()
        .map(|status| (status.pubkey.clone(), status))
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
