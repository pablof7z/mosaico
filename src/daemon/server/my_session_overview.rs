//! Identity-scoped presentation data for the current agent session.

use super::channel_membership_rpc::resolve_caller;
use super::*;
use crate::state::{MessageSearchQuery, MESSAGE_SEARCH_MAX_LIMIT};
use std::collections::HashMap;

#[derive(Clone)]
struct ChannelCard {
    channel_h: String,
    path: String,
    about: String,
}

#[derive(Clone)]
struct Interaction {
    pubkey: String,
    channel: String,
    message: String,
    created_at: u64,
    message_id: String,
}

pub(super) async fn rpc(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let rec = resolve_caller(state, params, "my session overview")?;
    let extension_live = super::session_delivery::extension_delivery_live(state, &rec);
    let now = crate::util::now_secs();
    let (presence, channels, interactions) = state.with_store(|store| {
        let presence =
            crate::session_presence::publication_with_extension(store, &rec, extension_live);
        let channel_ids = presence.channels.iter().cloned().collect::<Vec<_>>();
        let mut channels = channel_ids
            .iter()
            .filter_map(|channel_h| {
                let path = crate::channel_ref::full_channel_ref(store, channel_h);
                (!path.is_empty()).then(|| ChannelCard {
                    channel_h: channel_h.clone(),
                    path,
                    about: store
                        .get_channel(channel_h)
                        .ok()
                        .flatten()
                        .map(|channel| channel.about)
                        .unwrap_or_default(),
                })
            })
            .collect::<Vec<_>>();
        channels.sort_by(|left, right| left.path.cmp(&right.path));
        let channel_paths = channels
            .iter()
            .map(|channel| (channel.channel_h.clone(), channel.path.clone()))
            .collect::<HashMap<_, _>>();
        let interactions = latest_interactions(store, &rec.pubkey, &channel_ids, &channel_paths)?;
        Ok::<_, anyhow::Error>((presence, channels, interactions))
    })?;

    let peer_pubkeys = interactions
        .iter()
        .map(|interaction| interaction.pubkey.clone())
        .collect::<Vec<_>>();
    let names = crate::profile::resolve_names(state, &peer_pubkeys).await;
    let self_name = crate::profile::resolve_name(state, &rec.pubkey)
        .await
        .unwrap_or_else(|| state.session_instance(&rec).display_slug());
    let channels = channels
        .into_iter()
        .map(|channel| serde_json::json!({ "path": channel.path, "about": channel.about }))
        .collect::<Vec<_>>();
    let interactions = interactions
        .into_iter()
        .filter_map(|interaction| {
            let name = names.get(&interaction.pubkey)?;
            Some(serde_json::json!({
                "name": name,
                "channel": interaction.channel,
                "message": interaction.message,
                "age": crate::util::relative_time(interaction.created_at, now),
            }))
        })
        .collect::<Vec<_>>();

    Ok(serde_json::json!({
        "name": self_name,
        "state": presence.state,
        "title": presence.title,
        "status_age": crate::util::relative_time(presence.state_since, now),
        "channels": channels,
        "interactions": interactions,
    }))
}

fn latest_interactions(
    store: &Store,
    self_pubkey: &str,
    channel_ids: &[String],
    channel_paths: &HashMap<String, String>,
) -> Result<Vec<Interaction>> {
    let mut latest = HashMap::<(String, String), Interaction>::new();
    let outgoing = store.search_messages(&MessageSearchQuery {
        channels: channel_ids.to_vec(),
        from_pubkeys: vec![self_pubkey.to_string()],
        limit: MESSAGE_SEARCH_MAX_LIMIT,
        ..Default::default()
    })?;
    for hit in outgoing.hits {
        for recipient in hit.recipients {
            record_interaction(
                store,
                self_pubkey,
                &recipient.recipient_pubkey,
                &hit.message,
                channel_paths,
                &mut latest,
            );
        }
    }
    let incoming = store.search_messages(&MessageSearchQuery {
        channels: channel_ids.to_vec(),
        to_pubkeys: vec![self_pubkey.to_string()],
        limit: MESSAGE_SEARCH_MAX_LIMIT,
        ..Default::default()
    })?;
    for hit in incoming.hits {
        record_interaction(
            store,
            self_pubkey,
            &hit.message.author_pubkey,
            &hit.message,
            channel_paths,
            &mut latest,
        );
    }
    let mut interactions = latest.into_values().collect::<Vec<_>>();
    interactions.sort_by(|left, right| {
        (right.created_at, &right.message_id)
            .cmp(&(left.created_at, &left.message_id))
            .then_with(|| left.pubkey.cmp(&right.pubkey))
            .then_with(|| left.channel.cmp(&right.channel))
    });
    Ok(interactions)
}

fn record_interaction(
    store: &Store,
    self_pubkey: &str,
    pubkey: &str,
    message: &crate::state::Message,
    channel_paths: &HashMap<String, String>,
    latest: &mut HashMap<(String, String), Interaction>,
) {
    let is_agent = pubkey != self_pubkey
        && store
            .get_profile(pubkey)
            .ok()
            .flatten()
            .is_some_and(|profile| !profile.is_backend && !profile.agent_slug.trim().is_empty());
    let Some(channel) = channel_paths.get(&message.channel_h) else {
        return;
    };
    if !is_agent {
        return;
    }
    let candidate = Interaction {
        pubkey: pubkey.to_string(),
        channel: channel.clone(),
        message: crate::profile::rewrite_body_mentions(store, &message.body),
        created_at: message.created_at,
        message_id: message.message_id.clone(),
    };
    let key = (candidate.pubkey.clone(), message.channel_h.clone());
    if latest.get(&key).is_none_or(|current| {
        (candidate.created_at, &candidate.message_id) > (current.created_at, &current.message_id)
    }) {
        latest.insert(key, candidate);
    }
}

#[cfg(test)]
#[path = "my_session_overview/tests.rs"]
mod tests;
