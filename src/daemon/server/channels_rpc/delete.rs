use super::*;
use crate::domain::{AgentRef, ChatMessage};
use crate::session_presence;
use crate::session_state::SessionState;
use std::collections::BTreeSet;

pub(in crate::daemon::server) async fn rpc_channel_delete(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        channel: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("channel_delete params")?;
    let channel = resolve_target_channel(state, &p.channel)?;
    delete_channel(state, &channel).await
}

/// Delete a channel via kind:9008 after notifying online agents.
///
/// Hierarchy: refuses when direct children still exist so operators delete
/// leaves first rather than orphaning subtrees into accidental roots.
pub(in crate::daemon::server) async fn delete_channel(
    state: &Arc<DaemonState>,
    channel: &str,
) -> Result<serde_json::Value> {
    let channel_ref = state
        .with_store(|store| super::super::channel_resolve::channel_reference_for(store, channel))?;
    // Workspace roots are deletable once empty of children. Hierarchy still
    // requires leaves first so subtrees are not promoted into accidental roots.
    let children = state.with_store(|s| s.list_child_channels(channel))?;
    if !children.is_empty() {
        anyhow::bail!(
            "cannot delete {channel_ref}: it has {} child channel(s); delete children first",
            children.len()
        );
    }

    let online = online_agents_in_channel(state, channel)?;
    let notice_event_id = if online.is_empty() {
        String::new()
    } else {
        publish_deletion_notice(state, channel, &online).await?
    };

    let mgmt_keys = state.management_keys()?;
    let builder = crate::fabric::nip29::lifecycle::as_nostr(nmp_nip29::delete_group());
    let event_id = state
        .nmp()
        .publish_group(channel, builder, &mgmt_keys)?
        .to_hex();

    state.with_store(|s| s.purge_deleted_channel(channel))?;

    Ok(serde_json::json!({
        "channel": channel_ref,
        "event_id": event_id,
        "notice_event_id": notice_event_id,
        "notified_agents": online
            .iter()
            .map(|agent| serde_json::json!({
                "pubkey": agent.pubkey,
                "slug": agent.slug,
            }))
            .collect::<Vec<_>>(),
        "deleted": true,
    }))
}

#[derive(Clone, Debug)]
struct OnlineAgent {
    pubkey: String,
    slug: String,
}

fn online_agents_in_channel(state: &Arc<DaemonState>, channel: &str) -> Result<Vec<OnlineAgent>> {
    let now = crate::util::now_secs();
    let backend = state.backend_pubkey().unwrap_or_default();
    state.with_store(|store| {
        let members = store
            .list_channel_members(channel)?
            .into_iter()
            .map(|m| m.pubkey)
            .collect::<BTreeSet<_>>();
        let mut seen = BTreeSet::new();
        let mut online = Vec::new();
        for status in store.live_status_for_channel(channel, now)? {
            if !members.contains(&status.pubkey) {
                continue;
            }
            if status.pubkey == backend {
                continue;
            }
            if store
                .get_profile(&status.pubkey)?
                .is_some_and(|p| p.is_backend)
            {
                continue;
            }
            let presence = session_presence::remote(&status, now);
            if presence.state == SessionState::Offline || !presence.state.is_live() {
                continue;
            }
            if !seen.insert(status.pubkey.clone()) {
                continue;
            }
            let slug = if status.slug.trim().is_empty() {
                store
                    .resolve_slug_for_pubkey(&status.pubkey)?
                    .unwrap_or_else(|| crate::util::pubkey_short(&status.pubkey))
            } else {
                status.slug.clone()
            };
            // Humans and unnamed identities are not tagged as agents.
            if slug.trim().is_empty() {
                continue;
            }
            online.push(OnlineAgent {
                pubkey: status.pubkey,
                slug,
            });
        }
        online.sort_by(|a, b| a.slug.cmp(&b.slug).then(a.pubkey.cmp(&b.pubkey)));
        Ok(online)
    })
}

async fn publish_deletion_notice(
    state: &Arc<DaemonState>,
    channel: &str,
    agents: &[OnlineAgent],
) -> Result<String> {
    let keys = state.management_keys()?;
    let pubkey = keys.public_key().to_hex();
    let labels = agents
        .iter()
        .map(|a| format!("@{}", a.slug))
        .collect::<Vec<_>>()
        .join(" ");
    let body = format!(
        "This channel has been deleted.{tail}",
        tail = if labels.is_empty() {
            String::new()
        } else {
            format!(" {labels}")
        }
    );
    let chat = ChatMessage {
        from: AgentRef::new(pubkey, format!("{} (mosaico)", state.host())),
        channel: channel.to_string(),
        body,
        mentioned_pubkeys: agents.iter().map(|a| a.pubkey.clone()).collect(),
        attachments: Vec::new(),
    };
    let published = state.provider().publish_chat_checked(&chat, &keys).await?;
    Ok(published.event_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Status, Store};

    fn status(pubkey: &str, slug: &str, state: SessionState, now: u64) -> Status {
        Status {
            pubkey: pubkey.into(),
            channel_h: "chan".into(),
            slug: slug.into(),
            title: "work".into(),
            activity: String::new(),
            workspace: "root".into(),
            branch: String::new(),
            state,
            state_since: now,
            last_seen: now,
            updated_at: now,
            expiration: now + 60,
        }
    }

    #[test]
    fn live_status_projection_excludes_offline_state() {
        let store = Store::open_memory().expect("store");
        let now = 1_700_000_000u64;
        store
            .upsert_status(&status("agent-pk", "coder", SessionState::Idle, now))
            .unwrap();
        store
            .upsert_status(&status("offline-pk", "gone", SessionState::Offline, now))
            .unwrap();
        let live = store
            .live_status_for_channel("chan", now)
            .unwrap()
            .into_iter()
            .filter(|s| session_presence::remote(s, now).state.is_live())
            .map(|s| s.pubkey)
            .collect::<Vec<_>>();
        assert_eq!(live, vec!["agent-pk".to_string()]);
    }
}
