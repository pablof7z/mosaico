use super::chat_target::resolve_chat_target_provisioning;
use super::resolution::work_root_for;
use super::*;
use crate::fabric::provider::chat::{OutboundChatRecipient, OutboundChatRecord};
use crate::util::CHANNEL_MESSAGE_CHAR_LIMIT;
use anyhow::bail;

mod body;
mod mention_guard;
mod react;
mod recipient;
mod reply;
#[cfg(test)]
mod tests;

pub(in crate::daemon::server) use react::rpc_channel_react;
pub(in crate::daemon::server) use recipient::resolve_recipient;
pub(in crate::daemon::server) use reply::rpc_channel_reply;

#[derive(serde::Deserialize, Default)]
#[allow(dead_code)]
pub(in crate::daemon::server) struct ChannelSendParams {
    message: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    force: bool,
    #[serde(default)]
    harness_session: Option<String>,
    #[serde(default)]
    pty_session: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    long_message: bool,
}

fn chat_publish_scope(
    selected_destination: &str,
    pinned_destination: Option<&str>,
    mention_channel: Option<&str>,
) -> String {
    pinned_destination
        .or(mention_channel)
        .unwrap_or(selected_destination)
        .to_string()
}

pub(in crate::daemon::server) async fn rpc_channel_send(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: ChannelSendParams =
        serde_json::from_value(params.clone()).context("parsing channel_send params")?;
    mention_guard::check(&p.message, &p.tags, p.force)?;
    let mut anchor = CallerAnchor::from_params(params);
    anchor.group = None;
    let rec = resolve_session(state, &anchor)?;
    let id = identity::load_or_create(&config::edge_home(), &rec.agent_slug, now_secs())?;
    let durable_pubkey = id.pubkey_hex();
    let target =
        resolve_chat_target_provisioning(state, &rec, p.channel.as_deref(), "channel send").await?;
    // `--channel` is destination selection only. When present it pins the event
    // to that channel; it never changes caller identity or message content.
    let destination = target.channel_h;
    let pinned_destination = target.explicit.then_some(destination.clone());
    let mut tagged = Vec::new();
    for raw in &p.tags {
        let label = raw.trim().trim_start_matches('@');
        if label.is_empty() {
            bail!("tag must not be empty");
        }
        let target = state
            .with_store(|s| resolve_recipient(s, &destination, &state.host, label))
            .with_context(|| format!("resolving --tag {raw:?}"))?;
        let same_work_root = state
            .with_store(|s| work_root_for(s, &destination) == work_root_for(s, &target.channel));
        if target.channel != destination && !same_work_root {
            bail!(
                "tagged agent is in channel {:?}, but this chat is for channel {:?}",
                target.channel,
                destination
            );
        }
        if tagged
            .iter()
            .any(|entry: &TaggedRecipient| entry.pubkey == target.pubkey)
        {
            continue;
        }
        tagged.push(TaggedRecipient {
            label: label.to_string(),
            pubkey: target.pubkey,
            run_id: target.target_run_id,
            channel: target.channel,
        });
    }
    let mentioned_pubkeys = tagged
        .iter()
        .map(|target| target.pubkey.clone())
        .collect::<Vec<_>>();
    let mentioned_labels = tagged
        .iter()
        .map(|target| target.label.clone())
        .collect::<Vec<_>>();
    let publish_scope = chat_publish_scope(
        &destination,
        pinned_destination.as_deref(),
        tagged.first().map(|target| target.channel.as_str()),
    );
    let body_to_send = body::format_tagged_body(&p.message, &tagged)?;
    if !p.long_message && body_to_send.chars().count() > CHANNEL_MESSAGE_CHAR_LIMIT {
        bail!(
            "your message is too long; keep it under {CHANNEL_MESSAGE_CHAR_LIMIT} characters or pass --long-message"
        );
    }
    // Local visibility and inbox routing must use the same channel as the signed
    // event's `h` tag. Otherwise relay readback of our own event can disagree
    // with the locally-seeded row and the primary-key de-dupe preserves the wrong
    // scope.
    let deliver_scope = publish_scope.clone();

    // Sign + label from the session's own minted identity.
    let instance = state.session_instance(&rec);
    let chat_signing_keys = state.session_signing_keys(&rec.agent_pubkey)?;
    let from_pubkey = instance.pubkey.clone();

    let chat = ChatMessage {
        from: instance.agent_ref(),
        channel: publish_scope.clone(),
        body: body_to_send.clone(),
        mentioned_pubkeys: mentioned_pubkeys.clone(),
    };
    let published = state
        .provider
        .publish_chat_checked(
            &chat,
            &chat_signing_keys,
            &OutboundChatRecord {
                channel_h: deliver_scope.clone(),
                body: body_to_send.clone(),
                recipients: tagged
                    .iter()
                    .map(|target| OutboundChatRecipient::new(target.pubkey.clone()))
                    .collect(),
                created_at: Some(now_secs()),
                direction: "outbound",
            },
        )
        .await?;
    let event_id = published.event_id;
    let created_at = published.created_at;
    note_explicit_chat_published(state, &rec.session_id, created_at);

    // Local live delivery: relays often don't echo an event back to the same
    // connection that published it. Seed the verbatim log and park inbox rows for
    // sessions already alive in the same routing scope.
    let routed = state.with_store(|s| {
        let mut routed = false;
        // Best-effort local delivery (the publish already succeeded), but a store
        // failure listing targets must not silently drop a direct mention — log it
        // loudly and skip local routing this call rather than abort.
        let targets = match s.list_alive_sessions() {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(
                    event_id = %event_id,
                    channel = %deliver_scope,
                    error = %e,
                    "channel_send: listing live sessions for local delivery failed — direct mention may not reach a local inbox/doorbell"
                );
                Vec::new()
            }
        };
        for target in targets {
            let is_direct_target = tagged.iter().any(|recipient| {
                recipient.run_id.as_deref() == Some(target.session_id.as_str())
            });
            let joined_target = s
                .is_session_joined_channel(&target.session_id, &deliver_scope)
                .unwrap_or(target.channel_h == deliver_scope);
            if !is_direct_target && !joined_target {
                continue;
            }
            if target.created_at > created_at {
                continue;
            }
            // Skip sender's own sessions by pubkey.
            if target.agent_pubkey == durable_pubkey || target.agent_pubkey == from_pubkey {
                continue;
            }
            // Only ring the doorbell for explicitly mentioned sessions/pubkeys;
            // channel-broadcast messages stay in relay_events for ambient context.
            let is_mentioned = is_direct_target
                || mentioned_pubkeys
                    .iter()
                    .any(|pubkey| pubkey == &target.agent_pubkey);
            if !is_mentioned {
                continue;
            }
            let enqueued = match s.enqueue_inbox(
                &event_id,
                &target.agent_pubkey,
                &from_pubkey,
                &deliver_scope,
                &body_to_send,
                created_at,
            ) {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!(
                        event_id = %event_id,
                        session = %target.session_id,
                        channel = %deliver_scope,
                        error = %e,
                        "channel_send: enqueue_inbox failed — this direct mention may never reach the target's inbox/doorbell"
                    );
                    false
                }
            };
            if enqueued {
                routed = true;
            }
            if let Err(e) =
                s.add_message_recipient(&event_id, &target.agent_pubkey, None)
            {
                tracing::error!(
                    event_id = %event_id,
                    session = %target.session_id,
                    channel = %deliver_scope,
                    error = %e,
                    "channel_send: recipient pubkey edge upsert failed"
                );
            }
        }
        routed
    });
    if routed {
        crate::session_host::ring_doorbells(state.clone());
    }

    let from_label = instance.display_slug();
    state.emit_tail(TailEvent::Msg {
        ts: created_at,
        channel: deliver_scope.clone(),
        from: from_label,
        to: if mentioned_pubkeys.is_empty() {
            "channel-chat".to_string()
        } else {
            mentioned_pubkeys
                .iter()
                .map(|pubkey| pubkey_short(pubkey))
                .collect::<Vec<_>>()
                .join(",")
        },
        body: body_to_send.chars().take(200).collect(),
    });

    Ok(serde_json::json!({
        "event_id": event_id,
        "channel": publish_scope,
        "mentioned_pubkeys": mentioned_pubkeys,
        "mentioned_labels": mentioned_labels,
    }))
}

pub(super) fn note_explicit_chat_published(state: &Arc<DaemonState>, session_id: &str, at: u64) {
    if let Err(e) = state.with_store(|s| s.mark_session_explicit_chat_published(session_id, at)) {
        tracing::warn!(
            session_id,
            error = %e,
            "channel_send: failed to persist explicit-publish marker; using in-memory auto-reply guard"
        );
    }
    auto_reply::note_explicit_publish(session_id);
}

pub(super) struct TaggedRecipient {
    label: String,
    pubkey: String,
    run_id: Option<String>,
    channel: String,
}
