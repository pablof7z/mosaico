use super::messages::message_rows;
use super::model::*;
use super::people::{member_rows, presence_rows};
use super::refs::display_name;
use super::{FabricContextInput, FabricMessageSeed};
use crate::state::{Session, Store};
use crate::util::relative_time;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub(super) fn build_view(store: &Store, input: FabricContextInput<'_>) -> FabricView {
    let root = project_root(store, input.scope);
    let project = channel_summary(store, &root);
    let mut channels = channels_for(store, input.session, input.scope);
    let forced_by_channel = group_forced(input.forced_messages, input.scope);
    for ch in forced_by_channel.keys() {
        if !channels.iter().any(|c| c == ch) {
            channels.push(ch.clone());
        }
    }
    channels.sort();
    channels.dedup();

    let mut view = FabricView {
        self_row: input.session.map(|s| SelfRow {
            agent: input.self_slug.to_string(),
            backend: input.local_host.to_string(),
            session_id: s.session_id.clone(),
        }),
        project,
        agents: agents(input.edge_home, input.cursor, input.now),
        channels: Vec::new(),
        inactive: inactive_channels(store, &root, &channels, input.now),
        important: Vec::new(),
        warnings: input
            .warnings
            .iter()
            .cloned()
            .map(|text| WarningRow { text })
            .collect(),
    };

    for channel in channels {
        let forced = forced_by_channel.get(&channel).cloned().unwrap_or_default();
        let messages = if input.session.is_some() {
            message_rows(store, &input, &channel, &forced)
        } else {
            (Vec::new(), 0)
        };
        let presence = presence_rows(store, &channel, &input);
        let full = input.cursor == 0;
        if !full && messages.0.is_empty() && presence.is_empty() {
            continue;
        }
        for msg in &messages.0 {
            if msg.mention {
                view.important.push(ImportantRow {
                    channel: msg.channel.clone(),
                    message_id: msg.id.clone(),
                });
            }
        }
        let summary = channel_summary(store, &channel);
        view.channels.push(ChannelBlock {
            id: channel.clone(),
            name: summary.name,
            about: summary.about,
            active: channel == input.scope,
            members: if full {
                member_rows(store, &channel, &input)
            } else {
                Vec::new()
            },
            presence,
            subchannels: if full {
                subchannel_rows(store, &channel)
            } else {
                Vec::new()
            },
            messages: messages.0,
            omitted: messages.1,
        });
    }
    view
}

fn channels_for(store: &Store, session: Option<&Session>, scope: &str) -> Vec<String> {
    let Some(rec) = session else {
        return vec![scope.to_string()];
    };
    let mut channels = store
        .list_session_joined_channels(&rec.session_id)
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

fn subchannel_rows(store: &Store, channel: &str) -> Vec<ChannelSummaryRow> {
    store
        .list_channels()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.parent == channel)
        .map(|c| {
            let name = c.human_name().unwrap_or(&c.channel_h).to_string();
            ChannelSummaryRow {
                name,
                about: c.about,
            }
        })
        .filter(|c| !c.name.is_empty())
        .collect()
}

fn inactive_channels(
    store: &Store,
    root: &str,
    active_channels: &[String],
    now: u64,
) -> Vec<InactiveChannelRow> {
    let active = active_channels.iter().cloned().collect::<BTreeSet<_>>();
    store
        .list_channels()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.parent == root && !active.contains(&c.channel_h))
        .filter_map(|c| {
            let name = c.human_name().unwrap_or(&c.channel_h).to_string();
            (!name.is_empty()).then(|| InactiveChannelRow {
                name,
                about: c.about,
                last_active: relative_time(c.updated_at, now),
            })
        })
        .take(12)
        .collect()
}

fn agents(edge_home: Option<&Path>, cursor: u64, now: u64) -> Vec<AgentRow> {
    let Some(edge_home) = edge_home else {
        return Vec::new();
    };
    crate::identity::list_invitable_agents(edge_home)
        .into_iter()
        .filter(|(_, _, created_at)| cursor == 0 || (*created_at > cursor && *created_at <= now))
        .map(|(slug, byline, _)| AgentRow {
            reference: slug,
            about: byline.unwrap_or_default(),
        })
        .collect()
}

fn project_root(store: &Store, channel: &str) -> String {
    store
        .channel_project_root(channel)
        .ok()
        .flatten()
        .unwrap_or_else(|| channel.to_string())
}

fn channel_summary(store: &Store, channel: &str) -> ProjectRow {
    let ch = store.get_channel(channel).ok().flatten();
    ProjectRow {
        name: ch
            .as_ref()
            .and_then(|c| c.human_name())
            .map(str::to_string)
            .unwrap_or_else(|| display_name(store, channel)),
        about: ch.map(|c| c.about).unwrap_or_default(),
    }
}
