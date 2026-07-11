use super::chat_target::resolve_chat_target_provisioning;
use super::resolution::work_root_for;
use super::*;
use crate::fabric::provider::chat::OutboundChatRecord;
use crate::util::CHANNEL_MESSAGE_CHAR_LIMIT;
use anyhow::bail;

mod body;
mod recipient;
mod reply;
#[cfg(test)]
mod tests;

pub(in crate::daemon::server) use recipient::resolve_recipient;
pub(in crate::daemon::server) use reply::rpc_channel_reply;

#[derive(serde::Deserialize, Default)]
#[allow(dead_code)]
pub(in crate::daemon::server) struct ChannelSendParams {
    message: String,
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
    if long_message_requires_override(&p) {
        bail!(
            "your message is too long; keep it under {CHANNEL_MESSAGE_CHAR_LIMIT} characters or pass --long-message"
        );
    }
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
    // Mention target: the FIRST inline `@<agent-instance-label>` in the body that
    // resolves to a known instance pubkey in the selected destination. An
    // unresolvable token is silently treated as no mention — it must never bail
    // or block the chat.
    let mention_token = crate::idref::extract_mentions(&p.message)
        .into_iter()
        .next();
    let mention = if let Some(raw) = mention_token {
        match state.with_store(|s| resolve_recipient(s, &destination, &state.host, &raw)) {
            Ok(target) => {
                let same_work_root = state.with_store(|s| {
                    work_root_for(s, &destination) == work_root_for(s, &target.channel)
                });
                if target.channel != destination && !same_work_root {
                    anyhow::bail!(
                        "mention target is in channel {:?}, but this chat is for channel {:?}",
                        target.channel,
                        destination
                    );
                }
                Some((target.pubkey, target.target_session, target.channel, raw))
            }
            // An unknown token is an expected "no mention" (silent). A genuine
            // store failure underneath, however, silently DROPS a real mention —
            // surface that loudly so DB errors aren't mistaken for unknown handles.
            Err(e) => {
                handle_mention_resolution_error(&raw, e)?;
                None
            }
        }
    } else {
        None
    };
    let mentioned_pubkey = mention.as_ref().map(|(pk, ..)| pk.clone());
    let mentioned_session = mention.as_ref().and_then(|(_, sid, ..)| sid.clone());
    let mentioned_label = mention.as_ref().map(|(.., raw)| raw.clone());
    let publish_scope = chat_publish_scope(
        &destination,
        pinned_destination.as_deref(),
        mention.as_ref().map(|(_, _, channel, _)| channel.as_str()),
    );
    let body_to_send = mention
        .as_ref()
        .map(|(pk, _, _, raw)| body::rewrite_first_resolved_mention(&p.message, raw, pk))
        .unwrap_or_else(|| p.message.clone());
    // Local visibility and inbox routing must use the same channel as the signed
    // event's `h` tag. Otherwise relay readback of our own event can disagree
    // with the locally-seeded row and the primary-key de-dupe preserves the wrong
    // scope.
    let deliver_scope = publish_scope.clone();

    // Sign + label from the session's own minted identity.
    let instance = state.session_instance(&rec);
    let chat_signing_keys = state.session_signing_keys(&rec.session_id)?;
    let from_pubkey = instance.pubkey.clone();

    let chat = ChatMessage {
        from: instance.agent_ref(),
        channel: publish_scope.clone(),
        body: body_to_send.clone(),
        mentioned_pubkey: mentioned_pubkey.clone(),
    };
    let published = state
        .provider
        .publish_chat_checked(
            &chat,
            &chat_signing_keys,
            &OutboundChatRecord {
                from_session: Some(rec.session_id.clone()),
                channel_h: deliver_scope.clone(),
                body: body_to_send.clone(),
                mentioned_pubkey: mentioned_pubkey.clone(),
                mentioned_session: mentioned_session.clone(),
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
            let is_direct_target = mentioned_session.as_deref() == Some(target.session_id.as_str());
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
                || mentioned_pubkey.as_deref() == Some(target.agent_pubkey.as_str());
            if !is_mentioned {
                continue;
            }
            let enqueued = match s.enqueue_inbox(
                &event_id,
                &target.session_id,
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
            if let Err(e) = s.add_message_recipient(
                &event_id,
                &target.agent_pubkey,
                Some(&target.session_id),
                None,
            ) {
                tracing::error!(
                    event_id = %event_id,
                    session = %target.session_id,
                    channel = %deliver_scope,
                    error = %e,
                    "channel_send: recipient session edge upsert failed"
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
        from_session: Some(rec.session_id),
        to: mentioned_pubkey
            .as_deref()
            .map(pubkey_short)
            .unwrap_or_else(|| "channel-chat".to_string()),
        to_session: mentioned_session.clone(),
        body: body_to_send.chars().take(200).collect(),
    });

    Ok(serde_json::json!({
        "event_id": event_id,
        "channel": publish_scope,
        "mentioned_pubkey": mentioned_pubkey,
        "mentioned_session": mentioned_session,
        "mentioned_label": mentioned_label,
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

fn long_message_requires_override(p: &ChannelSendParams) -> bool {
    !p.long_message && p.message.chars().count() > CHANNEL_MESSAGE_CHAR_LIMIT
}

fn handle_mention_resolution_error(raw: &str, e: anyhow::Error) -> Result<()> {
    if e.chain().any(|c| c.is::<rusqlite::Error>()) {
        anyhow::bail!("failed to resolve mention @{raw}: {e:#}");
    }
    Ok(())
}
