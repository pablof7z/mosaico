use std::collections::BTreeMap;

use super::status_text;
use crate::fabric_context::capture::{MembersInput, StatusCap, ViewInputs};
use crate::fabric_context::model::MemberRow;
use crate::util::relative_time;

/// Full-snapshot member rows from the frozen roster, profile, and status inputs.
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
        .map(|(pk, _role)| {
            let status = status_map.get(&pk);
            MemberRow {
                reference: reference(inputs, &pk, status),
                agent_slug: agent_slug(inputs, &pk),
                status: status
                    .map(|s| status_text(s))
                    .unwrap_or_else(|| "offline".to_string()),
                seen: status
                    .map(|s| relative_time(s.last_seen, now))
                    .unwrap_or_else(|| "unknown".to_string()),
            }
        })
        .collect()
}

/// Live statuses keyed by pubkey, preserving the updated_at DESC last insert.
fn live_status_map(statuses: &[StatusCap], now: u64) -> BTreeMap<String, &StatusCap> {
    statuses
        .iter()
        .filter(|s| s.expiration >= now)
        .map(|s| (s.pubkey.clone(), s))
        .collect()
}

fn reference(inputs: &ViewInputs, pk: &str, status: Option<&&StatusCap>) -> String {
    if pk == inputs.meta.self_pubkey {
        return inputs.meta.self_ref.clone();
    }
    member_reference(&inputs.members, &inputs.meta.local_host, pk, status)
}

fn agent_slug(inputs: &ViewInputs, pk: &str) -> String {
    if pk == inputs.meta.self_pubkey {
        return inputs
            .meta
            .self_row
            .as_ref()
            .map(|s| s.agent_slug.clone())
            .unwrap_or_default();
    }
    inputs
        .members
        .agent_slugs
        .get(pk)
        .cloned()
        .unwrap_or_default()
}

fn member_reference(
    members: &MembersInput,
    _meta_local_host: &str,
    pk: &str,
    status: Option<&&StatusCap>,
) -> String {
    if let Some(s) = status.filter(|s| !s.session_id.is_empty()) {
        let profile_agent_slug = members
            .agent_slugs
            .get(pk)
            .map(String::as_str)
            .unwrap_or("");
        return crate::fabric_context::refs::session_ref(
            &s.session_id,
            &s.slug,
            profile_agent_slug,
        );
    }
    members.refs.get(pk).cloned().unwrap_or_default()
}
