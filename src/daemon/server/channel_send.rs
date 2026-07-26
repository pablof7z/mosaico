use super::chat_target::resolve_chat_target;
use super::resolution::work_root_for;
use super::*;
use crate::fabric::provider::chat::OutboundChatRecord;
use crate::util::CHANNEL_MESSAGE_CHAR_LIMIT;
use anyhow::bail;

mod body;
mod mention_guard;
mod react;
mod recipient;
mod recipient_notice;
mod reply;
mod self_target;
#[cfg(test)]
mod tests;

pub(in crate::daemon::server) use react::rpc_channel_react;
pub(in crate::daemon::server) use recipient::resolve_recipient;
use recipient::TaggedRecipient;
pub(in crate::daemon::server) use reply::rpc_channel_reply;

#[derive(serde::Deserialize, Default)]
#[allow(dead_code)]
pub(in crate::daemon::server) struct ChannelSendParams {
    message: String,
    #[serde(default)]
    attachments: Vec<crate::attachment::Attachment>,
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
    let anchor = CallerAnchor::from_params(params);
    let rec = resolve_session(state, &anchor)?;
    let target = resolve_chat_target(state, &rec, p.channel.as_deref(), "channel send")?;
    // `--channel` is destination selection only. When present it pins the event
    // to that channel; it never changes caller identity or message content.
    let destination = target.channel_h;
    super::mcp_actor::ensure_membership_if_actor(state, &rec, &destination).await?;
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
        self_target::reject(&rec.pubkey, &target.pubkey, self_target::Action::Tag(label))?;
        let same_work_root = state.with_store(|s| -> Result<bool> {
            Ok(work_root_for(s, &destination)? == work_root_for(s, &target.channel)?)
        })?;
        if target.channel != destination && !same_work_root {
            let (target_ref, destination_ref) = state.with_store(|store| {
                (
                    channel_resolve::channel_reference_for(store, &target.channel),
                    channel_resolve::channel_reference_for(store, &destination),
                )
            });
            bail!(
                "tagged agent is in channel {}, but this chat is for channel {}",
                target_ref?,
                destination_ref?
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
    let recipient_reminders = state
        .with_store(|store| recipient_notice::suspension_reminders(store, &tagged, now_secs()))?;
    let publish_scope = chat_publish_scope(
        &destination,
        pinned_destination.as_deref(),
        tagged.first().map(|target| target.channel.as_str()),
    );

    // Attachments and chat are signed by the same session identity. Uploads
    // finish before the NIP-29 event is built, so all consumers see final URLs.
    let instance = state.session_instance(&rec);
    let chat_signing_keys = state.session_signing_keys(&rec.pubkey)?;
    let from_pubkey = instance.pubkey.clone();
    let expanded_message = crate::attachment::upload_and_expand(
        &p.message,
        &p.attachments,
        &state.cfg.relays,
        &chat_signing_keys,
    )
    .await?;
    let body_to_send = body::format_tagged_body(&expanded_message, &tagged)?;
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
                direction: "outbound",
            },
        )
        .await?;
    let event_id = published.event_id;
    let created_at = published.created_at;
    // Relays need not echo a successful publish to this connection. Use the
    // same ownership router as inbound events so local direct delivery is
    // durable even when the target is stopped or has no channel route.
    super::direct_mentions::route(
        state,
        super::direct_mentions::DirectMention {
            event_id: &event_id,
            from_pubkey: &from_pubkey,
            channel_h: &deliver_scope,
            body: &body_to_send,
            created_at,
            target_pubkeys: &mentioned_pubkeys,
        },
    )?;

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

    let channel_ref = state
        .with_store(|store| super::channel_resolve::channel_reference_for(store, &publish_scope))?;
    Ok(serde_json::json!({
        "event_id": event_id,
        "channel": channel_ref,
        "mentioned_pubkeys": mentioned_pubkeys,
        "mentioned_labels": mentioned_labels,
        "recipient_reminders": recipient_reminders,
    }))
}
