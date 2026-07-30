//! Pure full/delta selection for the canonical agent fabric document.
//!
//! Cursor policy lives here. The XML renderer receives only the resulting
//! document and cannot distinguish `my session` from a hook invocation.

use std::collections::HashSet;

use super::capture::{EvCap, MsgBundle, StatusCap, ViewInputs};
use super::model::*;
use crate::util::relative_time;

mod members;
mod presence;
mod topology;

pub(in crate::fabric_context) use presence::{
    presence_rows, presence_snapshot_row, projected_presence,
};

pub(super) use members::member_row;
pub(crate) use members::missing_profile_pubkeys;

const WINDOW_SECS: u64 = 4 * 60 * 60;
const MAX_CLUSTER_GAP_SECS: u64 = 20 * 60;
const MAX_CLUSTER_ROWS: usize = 30;

pub(crate) fn assemble_view(inputs: &ViewInputs, cursor: u64, now: u64) -> FabricView {
    let full = cursor == 0;
    let (reactions, reactions_omitted) =
        super::reactions::group_reactions(&inputs.reactions.rows, cursor, now);
    let mut view = FabricView {
        now,
        self_row: inputs.meta.self_row.as_ref().map(self_row),
        hosts: host_rows(inputs, cursor, now, full),
        workspaces: topology::workspace_rows(inputs, cursor, now, full),
        important: Vec::new(),
        reactions,
        reactions_omitted,
        warnings: inputs
            .meta
            .warnings
            .iter()
            .cloned()
            .map(|text| WarningRow { text })
            .collect(),
        notices: Vec::new(),
    };
    collect_important(&mut view);
    if !full && !view.has_activity() && inputs.force() {
        view.notices.push(NoticeRow::NoNewActivity {
            workspace: inputs.meta.current_workspace.clone(),
        });
    }
    view
}

fn self_row(row: &super::capture::SelfCap) -> SelfRow {
    let hint = if row.title.is_empty() {
        "No session status set. Read mosaico://skill/public-work-status before \
         publishing a concise outcome that helps peers understand your work."
    } else if row.turn_count > 0 && row.turn_count.is_multiple_of(12) {
        "Is this session status still the meaningful outcome you own? Read \
         mosaico://skill/public-work-status before updating it."
    } else {
        ""
    };
    SelfRow {
        name: row.name.clone(),
        host: row.host.clone(),
        workspace: row.workspace.clone(),
        branch: row.branch.clone(),
        headless: row.headless,
        title: row.title.clone(),
        hint: hint.to_string(),
    }
}

fn host_rows(inputs: &ViewInputs, cursor: u64, now: u64, full: bool) -> Option<Vec<HostRow>> {
    let rows = inputs
        .meta
        .hosts
        .iter()
        .filter_map(|host| {
            let agents = host
                .agents
                .iter()
                .filter(|agent| full || (agent.created_at > cursor && agent.created_at <= now))
                .map(|agent| AgentRow {
                    reference: agent.reference.clone(),
                    about: crate::agent_about::for_injection(&agent.about),
                })
                .collect::<Vec<_>>();
            (full || !agents.is_empty()).then(|| HostRow {
                name: host.name.clone(),
                roots: host.roots.clone(),
                agents,
            })
        })
        .collect::<Vec<_>>();
    (full || !rows.is_empty()).then_some(rows)
}

fn collect_important(view: &mut FabricView) {
    let Some(workspaces) = &view.workspaces else {
        return;
    };
    for channel in workspaces.iter().flat_map(workspace_channels) {
        for message in &channel.messages {
            if message.mention {
                view.important.push(ImportantRow {
                    channel_ref: message.channel_ref.clone(),
                    message_id: message.id.clone(),
                });
            }
        }
    }
}

fn workspace_channels(workspace: &WorkspaceView) -> Vec<&ChannelBlock> {
    let mut rows = Vec::new();
    if let Some(root) = &workspace.root {
        collect_channels(root, &mut rows);
    }
    for channel in &workspace.channels {
        collect_channels(channel, &mut rows);
    }
    rows
}

fn collect_channels<'a>(channel: &'a ChannelBlock, rows: &mut Vec<&'a ChannelBlock>) {
    rows.push(channel);
    for child in &channel.children {
        collect_channels(child, rows);
    }
}

pub(super) fn message_rows(bundle: &MsgBundle, cursor: u64, now: u64) -> (Vec<MessageRow>, usize) {
    let since = if cursor == 0 {
        now.saturating_sub(WINDOW_SECS)
    } else {
        cursor
    };
    let mut events = bundle
        .events
        .iter()
        .filter(|event| event.created_at > since)
        .cloned()
        .collect::<Vec<_>>();
    let omitted = if cursor == 0 {
        let total = events.len();
        events = recent_cluster(events);
        total.saturating_sub(events.len())
    } else {
        0
    };
    let mut seen = events
        .iter()
        .map(|event| event.id.clone())
        .collect::<HashSet<_>>();
    for forced in &bundle.forced {
        if seen.insert(forced.id.clone()) {
            events.push(forced.clone());
        }
    }
    events.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    let forced_mentions = bundle
        .forced
        .iter()
        .filter(|event| event.forced_mention)
        .map(|event| event.id.as_str())
        .collect::<HashSet<_>>();
    let rows = events
        .into_iter()
        .map(|event| MessageRow {
            mention: event.mentions_self || forced_mentions.contains(event.id.as_str()),
            created_at: event.created_at,
            id: event.id,
            channel_ref: event.channel_ref,
            from: event.from_ref,
            recipients: event.recipient_refs,
            body: event.body,
            needs_reply_nudge: event.needs_reply_nudge,
        })
        .collect();
    (rows, omitted)
}

fn recent_cluster(mut events: Vec<EvCap>) -> Vec<EvCap> {
    if events.len() <= MAX_CLUSTER_ROWS {
        return events;
    }
    events.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    let mut start = events.len().saturating_sub(1);
    while start > 0 {
        let gap = events[start]
            .created_at
            .saturating_sub(events[start - 1].created_at);
        if gap > MAX_CLUSTER_GAP_SECS || events.len() - start >= MAX_CLUSTER_ROWS {
            break;
        }
        start -= 1;
    }
    events.split_off(start)
}
