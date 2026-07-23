use std::collections::BTreeMap;

use super::projected_presence;
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
        .filter_map(|(pk, _role)| {
            let status = status_map.get(&pk);
            let presence = status.map(|status| projected_presence(status, now));
            let state = presence
                .as_ref()
                .map(|row| row.state)
                .unwrap_or(crate::session_state::SessionState::Offline);
            let status_text = presence
                .as_ref()
                .map(crate::session_presence::PublicPresence::text)
                .unwrap_or_default();
            if status_text.is_empty() {
                return None;
            }
            Some(MemberRow {
                reference: reference(inputs, &pk, status),
                state,
                status: status_text,
                since: presence
                    .map(|row| relative_time(row.state_since, now))
                    .unwrap_or_else(|| "unknown".to_string()),
            })
        })
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
