use super::messages::message_rows;
use super::model::*;
use super::people::{member_rows, presence_rows};
use super::refs::display_name;
use super::{missing_channel_warning, FabricContextInput, FabricMessageSeed};
use crate::state::{Session, Store};
use crate::util::relative_time;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn build_view(store: &Store, input: FabricContextInput<'_>) -> FabricView {
    let root = project_root(store, input.scope);
    let project = project_summary(store, &root);
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

    let mut view = FabricView {
        self_row: input.session.map(|s| SelfRow {
            agent: input.self_slug.to_string(),
            backend: input.local_host.to_string(),
            session_id: s.session_id.clone(),
        }),
        project,
        agents: agents(store, &root, input.cursor, input.now, input.local_host),
        channels: Vec::new(),
        unjoined: unjoined_channels(store, &root, &channels, input.now),
        important: Vec::new(),
        warnings,
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
            name: summary.name,
            about: summary.about,
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
        .filter(|c| c.parent == channel && !c.is_archived())
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

fn unjoined_channels(
    store: &Store,
    root: &str,
    joined_channels: &[String],
    now: u64,
) -> Vec<UnjoinedChannelRow> {
    let joined = joined_channels.iter().cloned().collect::<BTreeSet<_>>();
    store
        .list_channels()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.parent == root && !joined.contains(&c.channel_h) && !c.is_archived())
        .filter_map(|c| {
            let name = c.human_name().unwrap_or(&c.channel_h).to_string();
            (!name.is_empty()).then(|| UnjoinedChannelRow {
                name,
                about: c.about,
                last_active: relative_time(c.updated_at, now),
            })
        })
        .take(12)
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

fn project_root(store: &Store, channel: &str) -> String {
    store
        .channel_project_root(channel)
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

fn channel_summary(store: &Store, channel: &str) -> ProjectRow {
    let ch = store
        .get_channel(channel)
        .ok()
        .flatten()
        .expect("renderable channels are filtered through get_channel first");
    ProjectRow {
        name: ch
            .human_name()
            .map(str::to_string)
            .unwrap_or_else(|| display_name(store, channel)),
        about: ch.about,
    }
}

fn project_summary(store: &Store, channel: &str) -> ProjectRow {
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
