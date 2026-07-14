use super::messages::message_rows;
use super::model::*;
use super::people::{member_rows, presence_rows};
use super::refs::display_name;
use super::{missing_channel_warning, FabricContextInput, FabricMessageSeed};
use crate::state::{Session, Store};
use std::collections::BTreeMap;

pub(super) fn build_view(store: &Store, input: FabricContextInput<'_>) -> FabricView {
    let root = root_channel(store, input.scope);
    let workspace = workspace_summary(store, &root);
    let mut warnings = input
        .warnings
        .iter()
        .cloned()
        .map(|text| WarningRow { text })
        .collect::<Vec<_>>();
    let mut channels = channels_for(store, input.session, input.scope);
    let forced_by_channel = group_forced(input.forced_messages, input.scope);
    for ch in forced_by_channel.keys() {
        if !channels.iter().any(|c| c == ch) {
            channels.push(ch.clone());
        }
    }
    channels.sort();
    channels.dedup();
    channels.retain(|channel| channel_ready_for_render(store, channel, &mut warnings));

    // Reactions on the caller's own recent messages (parity with the pure
    // `assemble_view` path — both share the same capture + grouping helpers).
    let reaction_floor = input.session.map(|s| s.created_at).unwrap_or(0);
    let reaction_sources = super::reactions::capture_reaction_sources(
        store,
        input.self_pubkey,
        reaction_floor,
        input.local_host,
        input.backend_pubkey,
    );
    let (reactions, reactions_omitted) =
        super::reactions::group_reactions(&reaction_sources, input.cursor, input.now);

    let mut view = FabricView {
        self_row: input.session.map(|s| SelfRow {
            agent: input.self_slug.to_string(),
            agent_slug: s.agent_slug.clone(),
            host: input.local_host.to_string(),
            work_topic: s
                .visible_work_topic(input.now)
                .unwrap_or_default()
                .to_string(),
        }),
        workspace,
        agents: agents(store, &root, input.cursor, input.now, input.local_host),
        root: None,
        channels: Vec::new(),
        other_workspaces: Vec::new(),
        important: Vec::new(),
        reactions,
        reactions_omitted,
        warnings,
        incremental: input.cursor != 0,
    };

    let mut channel_rows = Vec::new();
    for channel in channels {
        let forced = forced_by_channel.get(&channel).cloned().unwrap_or_default();
        let messages = if input.session.is_some() {
            message_rows(store, &input, &channel, &forced)
        } else {
            (Vec::new(), 0)
        };
        let presence = presence_rows(store, &channel, &input);
        let full = input.cursor == 0;
        let children = subchannel_rows(store, &channel, input.cursor, input.now);
        if !full && messages.0.is_empty() && presence.is_empty() && children.is_empty() {
            continue;
        }
        for msg in &messages.0 {
            if msg.mention {
                view.important.push(ImportantRow {
                    channel_ref: msg.channel_ref.clone(),
                    message_id: msg.id.clone(),
                });
            }
        }
        let summary = channel_summary(store, &channel);
        channel_rows.push(ChannelBlock {
            name: summary.name,
            reference: crate::channel_ref::full_channel_ref(store, &channel),
            about: summary.about,
            members: if full {
                member_rows(store, &channel, &input)
            } else {
                Vec::new()
            },
            presence,
            children,
            messages: messages.0,
            omitted: messages.1,
        });
    }
    (view.root, view.channels) = super::tree::arrange(&view.workspace.name, channel_rows);
    view.other_workspaces = other_workspace_activity(store, &root, &input);
    view
}

fn other_workspace_activity(
    store: &Store,
    current_root: &str,
    input: &FabricContextInput<'_>,
) -> Vec<WorkspaceActivity> {
    if input.cursor == 0 {
        return Vec::new();
    }
    let mut by_root: BTreeMap<String, Vec<ChannelBlock>> = BTreeMap::new();
    for channel in store
        .list_channels()
        .unwrap_or_default()
        .into_iter()
        .filter(|channel| !channel.is_archived())
    {
        let Some(root) = store.root_channel_of(&channel.channel_h).ok().flatten() else {
            continue;
        };
        if root == current_root {
            continue;
        }
        let presence = presence_rows(store, &channel.channel_h, input);
        if presence.is_empty() {
            continue;
        }
        let summary = channel_summary(store, &channel.channel_h);
        by_root.entry(root).or_default().push(ChannelBlock {
            name: summary.name,
            reference: summary.channel,
            about: summary.about,
            members: Vec::new(),
            presence,
            children: Vec::new(),
            messages: Vec::new(),
            omitted: 0,
        });
    }
    by_root
        .into_iter()
        .map(|(root_name, rows)| {
            let workspace = workspace_summary(store, &root_name);
            let (root, channels) = super::tree::arrange(&workspace.name, rows);
            WorkspaceActivity {
                workspace,
                root,
                channels,
            }
        })
        .collect()
}

fn channels_for(store: &Store, session: Option<&Session>, scope: &str) -> Vec<String> {
    let Some(rec) = session else {
        return vec![scope.to_string()];
    };
    let mut channels = store
        .list_session_joined_channels(&rec.pubkey)
        .unwrap_or_else(|_| vec![(rec.channel_h.clone(), rec.created_at)])
        .into_iter()
        .map(|(h, _)| h)
        .collect::<Vec<_>>();
    if !channels.iter().any(|h| h == scope) {
        channels.push(scope.to_string());
    }
    channels
}

fn group_forced(
    rows: &[FabricMessageSeed],
    fallback_scope: &str,
) -> BTreeMap<String, Vec<FabricMessageSeed>> {
    let mut out: BTreeMap<String, Vec<FabricMessageSeed>> = BTreeMap::new();
    for row in rows {
        let channel = if row.channel.is_empty() {
            fallback_scope
        } else {
            &row.channel
        };
        out.entry(channel.to_string())
            .or_default()
            .push(row.clone());
    }
    out
}

fn subchannel_rows(store: &Store, channel: &str, cursor: u64, now: u64) -> Vec<ChannelBlock> {
    store
        .list_channels()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.parent == channel && !c.is_archived())
        .filter(|c| cursor == 0 || (c.updated_at > cursor && c.updated_at <= now))
        .filter_map(|c| {
            let name = c.human_name().unwrap_or(&c.channel_h).to_string();
            (!name.is_empty()).then(|| {
                ChannelBlock::compact(
                    name,
                    crate::channel_ref::full_channel_ref(store, &c.channel_h),
                    c.about,
                )
            })
        })
        .collect()
}

pub(super) fn agents(
    store: &Store,
    root: &str,
    cursor: u64,
    now: u64,
    local_host: &str,
) -> Vec<AgentRow> {
    store
        .list_agent_roster_for_channel(root)
        .unwrap_or_default()
        .into_iter()
        .filter(|row| cursor == 0 || (row.updated_at > cursor && row.updated_at <= now))
        .map(|row| AgentRow {
            reference: if row.host.is_empty() || row.host == local_host {
                row.slug
            } else {
                format!("{}@{}", row.slug, row.host)
            },
            about: row.use_criteria,
        })
        .collect()
}

fn root_channel(store: &Store, channel: &str) -> String {
    store
        .root_channel_of(channel)
        .ok()
        .flatten()
        .unwrap_or_else(|| channel.to_string())
}

fn channel_ready_for_render(store: &Store, channel: &str, warnings: &mut Vec<WarningRow>) -> bool {
    match store.get_channel(channel) {
        Ok(Some(ch)) if !ch.is_archived() => true,
        Ok(Some(_)) => false,
        _ => {
            warnings.push(WarningRow {
                text: missing_channel_warning(channel),
            });
            false
        }
    }
}

fn channel_summary(store: &Store, channel: &str) -> WorkspaceRow {
    let ch = store
        .get_channel(channel)
        .ok()
        .flatten()
        .expect("renderable channels are filtered through get_channel first");
    WorkspaceRow {
        name: if ch.parent.is_empty() {
            channel.to_string()
        } else {
            ch.human_name()
                .map(str::to_string)
                .unwrap_or_else(|| display_name(store, channel))
        },
        channel: crate::channel_ref::full_channel_ref(store, channel),
        about: ch.about,
    }
}

fn workspace_summary(store: &Store, channel: &str) -> WorkspaceRow {
    let ch = store.get_channel(channel).ok().flatten();
    WorkspaceRow {
        name: channel.to_string(),
        channel: ch
            .as_ref()
            .map(|_| crate::channel_ref::full_channel_ref(store, channel))
            .unwrap_or_default(),
        about: ch.map(|channel| channel.about).unwrap_or_default(),
    }
}
