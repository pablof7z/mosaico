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
