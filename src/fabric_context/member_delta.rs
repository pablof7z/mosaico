//! Recover member deltas from consecutive frozen captures.
//!
//! Relay events can arrive after their signed timestamp has fallen behind the
//! session cursor. Cursor-only assembly would then suppress a roster or status
//! change forever. The hook cache can close that race by comparing what agents
//! could semantically see in its prior and current captures.

use std::collections::{BTreeMap, BTreeSet};

use super::capture::{StatusCap, ViewInputs};
use super::model::{FabricView, MemberRow, PresenceRow};
use tree::{add_member, add_presence, channel_mut};

mod tree;

pub(crate) fn inject_member_deltas(
    view: &mut FabricView,
    previous: &ViewInputs,
    current: &ViewInputs,
    previous_now: u64,
    now: u64,
) {
    let references = channel_references(previous, current);
    let visible = member_channels(current);
    for channel in &visible {
        let Some(reference) = references.get(channel).filter(|path| !path.is_empty()) else {
            continue;
        };
        inject_presence_changes(
            view,
            reference,
            channel,
            previous,
            current,
            previous_now,
            now,
        );
        inject_roster_changes(
            view,
            reference,
            channel,
            previous,
            current,
            previous_now,
            now,
        );
    }
    inject_departures(view, previous, current, previous_now, &references, &visible);
}

/// Rebuild the joined-channel roster after daemon presentation state is lost.
///
/// The durable awareness cursor survives a daemon restart, while this hook cache
/// deliberately does not. Re-baseline members only: replaying old chatter would
/// violate cursor delivery semantics, but omitting the roster leaves the agent
/// unaware of peers already present before the restart.
pub(crate) fn inject_member_snapshot(view: &mut FabricView, current: &ViewInputs, now: u64) {
    let references = channel_references(current, current);
    for channel in current
        .meta
        .joined_channels
        .iter()
        .filter(|channel| member_channel(current, channel))
    {
        let Some(reference) = references.get(channel).filter(|path| !path.is_empty()) else {
            continue;
        };
        let Some(roster) = current.members.roster.get(channel) else {
            continue;
        };
        for pubkey in roster.keys() {
            if pubkey == &current.meta.self_pubkey || current.members.backend.contains(pubkey) {
                continue;
            }
            if statuses_by_pubkey(current, channel)
                .get(pubkey.as_str())
                .is_some_and(|status| status.changed_at > now)
            {
                continue;
            }
            let Some(row) = super::assemble::member_row(current, channel, pubkey, now) else {
                continue;
            };
            add_member(view, reference, row);
        }
    }
}

fn inject_presence_changes(
    view: &mut FabricView,
    reference: &str,
    channel: &str,
    previous: &ViewInputs,
    current: &ViewInputs,
    previous_now: u64,
    now: u64,
) {
    let before = statuses_by_pubkey(previous, channel);
    let after = statuses_by_pubkey(current, channel);
    let previous_visible = member_channel(previous, channel);
    for (pubkey, status) in after {
        if pubkey == current.meta.self_pubkey {
            continue;
        }
        let current_row = super::assemble::presence_snapshot_row(current, status, now);
        let previous_row = previous_visible
            .then(|| before.get(pubkey))
            .flatten()
            .and_then(|row| super::assemble::presence_snapshot_row(previous, row, previous_now));
        let Some(current_row) = current_row else {
            continue;
        };
        if presence_semantics(previous_row.as_ref()) != presence_semantics(Some(&current_row)) {
            add_presence(view, reference, current_row);
        }
    }
}

fn inject_roster_changes(
    view: &mut FabricView,
    reference: &str,
    channel: &str,
    previous: &ViewInputs,
    current: &ViewInputs,
    previous_now: u64,
    now: u64,
) {
    let Some(after) = current.members.roster.get(channel) else {
        return;
    };
    let previous_visible = member_channel(previous, channel);
    for pubkey in after.keys() {
        if pubkey == &current.meta.self_pubkey || current.members.backend.contains(pubkey) {
            continue;
        }
        if statuses_by_pubkey(current, channel)
            .get(pubkey.as_str())
            .is_some_and(|status| status.changed_at > now)
        {
            continue;
        }
        let current_row = super::assemble::member_row(current, channel, pubkey, now);
        let previous_row = previous_visible
            .then(|| super::assemble::member_row(previous, channel, pubkey, previous_now))
            .flatten();
        let Some(current_row) = current_row else {
            continue;
        };
        if member_semantics(previous_row.as_ref()) != member_semantics(Some(&current_row)) {
            add_member(view, reference, current_row);
        }
    }
}

fn inject_departures(
    view: &mut FabricView,
    previous: &ViewInputs,
    current: &ViewInputs,
    previous_now: u64,
    references: &BTreeMap<String, String>,
    visible: &BTreeSet<String>,
) {
    for channel in previous
        .members
        .hydrated
        .intersection(&current.members.hydrated)
    {
        if !visible.contains(channel) {
            continue;
        }
        let Some(before) = previous.members.roster.get(channel) else {
            continue;
        };
        let Some(after) = current.members.roster.get(channel) else {
            continue;
        };
        let names = before
            .keys()
            .filter(|pubkey| !after.contains_key(*pubkey))
            .filter(|pubkey| !previous.members.backend.contains(*pubkey))
            .filter_map(|pubkey| {
                super::assemble::member_row(previous, channel, pubkey, previous_now)
                    .map(|row| row.name.trim_start_matches('@').to_string())
                    .filter(|name| !name.is_empty())
            })
            .collect::<Vec<_>>();
        let Some(reference) = references.get(channel).filter(|path| !path.is_empty()) else {
            continue;
        };
        if !names.is_empty() {
            channel_mut(view, reference).departures.extend(names);
        }
    }
}

fn member_channels(inputs: &ViewInputs) -> BTreeSet<String> {
    if inputs.meta.self_row.is_none() {
        return inputs
            .meta
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.channels.iter().map(|channel| channel.h.clone()))
            .collect();
    }
    inputs
        .meta
        .joined_channels
        .iter()
        .filter(|channel| member_channel(inputs, channel))
        .cloned()
        .collect()
}

fn member_channel(inputs: &ViewInputs, channel: &str) -> bool {
    inputs.meta.joined_channels.contains(channel)
        && inputs.members.hydrated.contains(channel)
        && inputs
            .members
            .roster
            .get(channel)
            .is_some_and(|roster| roster.contains_key(&inputs.meta.self_pubkey))
}

fn channel_references(previous: &ViewInputs, current: &ViewInputs) -> BTreeMap<String, String> {
    current
        .meta
        .workspaces
        .iter()
        .chain(previous.meta.workspaces.iter())
        .flat_map(|workspace| workspace.channels.iter())
        .map(|channel| (channel.h.clone(), channel.reference.clone()))
        .collect()
}

fn statuses_by_pubkey<'a>(
    inputs: &'a ViewInputs,
    channel: &str,
) -> BTreeMap<&'a str, &'a StatusCap> {
    let roster = inputs.members.roster.get(channel);
    inputs
        .presence
        .statuses
        .get(channel)
        .into_iter()
        .flatten()
        .filter(|status| roster.is_some_and(|members| members.contains_key(status.pubkey.as_str())))
        .map(|status| (status.pubkey.as_str(), status))
        .collect()
}

fn member_semantics(
    row: Option<&MemberRow>,
) -> Option<(&str, &str, &str, &str, u8, Option<&str>, &str)> {
    row.map(|row| {
        (
            row.name.as_str(),
            row.host.as_str(),
            row.workspace.as_str(),
            row.branch.as_str(),
            match row.kind {
                super::model::MemberKind::Agent => 1,
                super::model::MemberKind::Human => 2,
            },
            row.state.map(|state| state.as_str()),
            row.status.as_str(),
        )
    })
}

fn presence_semantics(
    row: Option<&PresenceRow>,
) -> Option<(&str, &str, &str, &str, &str, &str, Option<(&str, &str)>)> {
    row.map(|row| {
        (
            row.name.as_str(),
            row.host.as_str(),
            row.workspace.as_str(),
            row.branch.as_str(),
            row.state.as_str(),
            row.status.as_str(),
            row.native_failure
                .as_ref()
                .map(|failure| (failure.outcome.as_str(), failure.message.as_str())),
        )
    })
}
