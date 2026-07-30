use super::super::*;
use super::recipient_notice;
use super::self_target;
use crate::fabric::provider::chat::OutboundChatRecord;
use anyhow::{bail, Context, Result};
use nostr::{PublicKey, ToBech32};

#[derive(serde::Deserialize, Default)]
struct ChannelReplyParams {
    id: String,
    message: String,
    #[serde(default)]
    attachments: Vec<crate::attachment::Attachment>,
}

fn parse_params(params: &serde_json::Value) -> Result<ChannelReplyParams> {
    super::params::validate_reply(params)?;
    serde_json::from_value(params.clone()).context("parsing channel_reply params")
}

pub(in crate::daemon::server) async fn rpc_channel_reply(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p = parse_params(params)?;
    if p.id.trim().is_empty() {
        bail!("reply id must not be empty");
    }
    if p.message.trim().is_empty() {
        bail!("reply message must not be empty");
    }
    let prepared_message = super::prepare_outbound_message(&p.message, &p.attachments)?;
    let rec = resolve_session(state, &CallerAnchor::from_params(params))?;
    let original = state
        .with_store(|s| s.get_message_by_prefix(p.id.trim()))
        .with_context(|| format!("resolving reply id {:?}", p.id.trim()))?
        .with_context(|| format!("message not found for reply id {:?}", p.id.trim()))?;
    self_target::reject(
        &rec.pubkey,
        &original.author_pubkey,
        self_target::Action::Reply,
    )?;
    let reply_to = original
        .native_event_id
        .clone()
        .unwrap_or_else(|| original.message_id.clone());
    let instance = state.session_instance(&rec);
    let keys = state.session_signing_keys(&rec.pubkey)?;
    let uploaded_attachments =
        crate::attachment::upload_all(&p.attachments, &state.cfg.relays, &keys).await?;
    let body = reply_body(&original.author_pubkey, &prepared_message)?;
    let recipient_reminders = state.with_store(|store| {
        recipient_notice::reply_suspension_reminders(store, &original, now_secs())
    })?;
    let chat = ChatMessage {
        from: instance.agent_ref(),
        channel: original.channel_h.clone(),
        body: body.clone(),
        mentioned_pubkeys: vec![original.author_pubkey.clone()],
        attachments: uploaded_attachments.clone(),
    };
    let published = state
        .provider
        .publish_chat_reply_checked(
            &chat,
            &reply_to,
            &keys,
            &OutboundChatRecord {
                channel_h: original.channel_h.clone(),
                direction: "outbound",
            },
        )
        .await?;
    state.record_coordination_action(&rec);
    let local_directory = match crate::attachment_receive::copy_local(
        &state.cfg.attachment_receive_directory,
        &published.event_id,
        &p.attachments,
    ) {
        Ok(directory) => directory,
        Err(error) => {
            tracing::warn!(
                event_id = published.event_id,
                %error,
                "local attachment copy failed; delivering ordinary message without files"
            );
            None
        }
    };
    super::persist_attachment_directory_then_deliver(
        &published.event_id,
        || match local_directory.as_ref() {
            Some(directory) => state.with_store(|store| {
                store.set_message_attachment_dir(&published.event_id, directory)
            }),
            None => Ok(false),
        },
        || {
            super::super::direct_mentions::route(
                state,
                super::super::direct_mentions::DirectMention {
                    event_id: &published.event_id,
                    from_pubkey: &rec.pubkey,
                    channel_h: &original.channel_h,
                    body: &body,
                    created_at: published.created_at,
                    target_pubkeys: std::slice::from_ref(&original.author_pubkey),
                    attachments: &uploaded_attachments,
                },
            )
        },
    )?;
    state.emit_tail(TailEvent::Msg {
        ts: published.created_at,
        channel: original.channel_h.clone(),
        from: instance.display_slug(),
        to: pubkey_short(&original.author_pubkey),
        body: body.chars().take(200).collect(),
    });

    let channel_ref = state
        .with_store(|store| channel_resolve::channel_reference_for(store, &original.channel_h))?;
    Ok(serde_json::json!({
        "event_id": published.event_id,
        "reply_to": reply_to,
        "channel": channel_ref,
        "mentioned_pubkey": original.author_pubkey,
        "recipient_reminders": recipient_reminders,
    }))
}

fn reply_body(author_pubkey: &str, message: &str) -> Result<String> {
    let pk = PublicKey::parse(author_pubkey)
        .with_context(|| format!("invalid author pubkey for reply: {author_pubkey}"))?;
    Ok(format!("nostr:{}: {message}", pk.to_bech32()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_channel_reply_rpc_field_is_rejected() {
        let error = match parse_params(&serde_json::json!({
            "id": "abc123",
            "message": "hello",
            "long_message": true,
        })) {
            Ok(_) => panic!("unknown channel_reply field was accepted"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("channel_reply received unknown field \"long_message\""));
    }
}
