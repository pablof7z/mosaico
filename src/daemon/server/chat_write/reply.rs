use super::super::*;
use crate::fabric::provider::chat::OutboundChatRecord;
use crate::state::{Message, Session};
use crate::util::CHAT_WRITE_CHAR_LIMIT;
use anyhow::{bail, Context, Result};
use nostr_sdk::prelude::{PublicKey, ToBech32};

#[derive(serde::Deserialize, Default)]
struct ChatReplyParams {
    id: String,
    message: String,
    #[serde(default)]
    long_message: bool,
}

pub(in crate::daemon::server) async fn rpc_chat_reply(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: ChatReplyParams =
        serde_json::from_value(params.clone()).context("parsing chat_reply params")?;
    if p.id.trim().is_empty() {
        bail!("reply id must not be empty");
    }
    if p.message.trim().is_empty() {
        bail!("reply message must not be empty");
    }
    if !p.long_message && p.message.chars().count() > CHAT_WRITE_CHAR_LIMIT {
        bail!(
            "your message is too long; keep it under {CHAT_WRITE_CHAR_LIMIT} characters or pass --long-message"
        );
    }

    let rec = resolve_session(state, &CallerAnchor::from_params(params))?;
    let original = state
        .with_store(|s| s.get_message_by_prefix(p.id.trim()))
        .with_context(|| format!("resolving reply id {:?}", p.id.trim()))?
        .with_context(|| format!("message not found for reply id {:?}", p.id.trim()))?;
    let reply_to = original
        .native_event_id
        .clone()
        .unwrap_or_else(|| original.message_id.clone());
    let body = reply_body(&original.author_pubkey, &p.message)?;

    let instance = state.session_instance(&rec);
    let keys = state.session_signing_keys(&rec.session_id)?;
    let chat = ChatMessage {
        from: instance.agent_ref(),
        channel: original.channel_h.clone(),
        body: body.clone(),
        mentioned_pubkey: Some(original.author_pubkey.clone()),
    };
    let published = state
        .provider
        .publish_chat_reply_checked(
            &chat,
            &reply_to,
            &keys,
            &OutboundChatRecord {
                from_session: Some(rec.session_id.clone()),
                channel_h: original.channel_h.clone(),
                body: body.clone(),
                mentioned_pubkey: Some(original.author_pubkey.clone()),
                mentioned_session: original.author_session.clone(),
                created_at: Some(now_secs()),
                direction: "outbound",
            },
        )
        .await?;
    auto_reply::note_published(&rec.session_id);
    enqueue_local_reply(
        state,
        &rec,
        &original,
        &published.event_id,
        &body,
        published.created_at,
    );
    state.emit_tail(TailEvent::Msg {
        ts: published.created_at,
        channel: original.channel_h.clone(),
        from: instance.display_slug(),
        from_session: Some(rec.session_id),
        to: pubkey_short(&original.author_pubkey),
        to_session: original.author_session.clone(),
        body: body.chars().take(200).collect(),
    });

    Ok(serde_json::json!({
        "event_id": published.event_id,
        "reply_to": reply_to,
        "channel": original.channel_h,
        "mentioned_pubkey": original.author_pubkey,
        "mentioned_session": original.author_session,
    }))
}

fn reply_body(author_pubkey: &str, message: &str) -> Result<String> {
    let pk = PublicKey::parse(author_pubkey)
        .with_context(|| format!("invalid author pubkey for reply: {author_pubkey}"))?;
    Ok(format!("nostr:{}: {message}", pk.to_bech32()?))
}

fn enqueue_local_reply(
    state: &Arc<DaemonState>,
    rec: &Session,
    original: &Message,
    event_id: &str,
    body: &str,
    created_at: u64,
) {
    let targets = state
        .with_store(|s| s.list_alive_sessions())
        .unwrap_or_default();
    let mut routed = false;
    state.with_store(|s| {
        for target in targets {
            if target.session_id == rec.session_id {
                continue;
            }
            let is_author_session =
                original.author_session.as_deref() == Some(target.session_id.as_str());
            let is_author_pubkey = target.agent_pubkey == original.author_pubkey;
            if !is_author_session && !is_author_pubkey {
                continue;
            }
            let joined = s
                .is_session_joined_channel(&target.session_id, &original.channel_h)
                .unwrap_or(target.channel_h == original.channel_h);
            if !joined {
                continue;
            }
            if s.enqueue_inbox(
                event_id,
                &target.session_id,
                &rec.agent_pubkey,
                &original.channel_h,
                body,
                created_at,
            )
            .unwrap_or(false)
            {
                routed = true;
            }
        }
    });
    if routed {
        crate::session_host::ring_doorbells(state.clone());
    }
}
