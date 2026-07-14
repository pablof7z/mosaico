use super::*;
use crate::fabric::provider::chat::OutboundChatRecipient;
use anyhow::Context;
use nostr_sdk::prelude::{PublicKey, ToBech32};
use std::collections::HashSet;
use std::time::Duration;

#[derive(serde::Deserialize)]
pub(super) struct DispatchParams {
    target: String,
    workspace: String,
    #[serde(default)]
    channels: Vec<String>,
    message: String,
}

pub(super) async fn rpc_dispatch(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: DispatchParams = serde_json::from_value(params.clone()).context("dispatch params")?;
    if p.workspace.trim().is_empty() {
        anyhow::bail!("dispatch requires --workspace");
    }
    if p.message.trim().is_empty() {
        anyhow::bail!("dispatch requires --message");
    }
    let target = crate::idref::parse_agent_backend_ref(&p.target).with_context(|| {
        format!(
            "malformed target {:?}: expected agent[@backend-label]",
            p.target
        )
    })?;
    let caller = resolve_session_inner(
        state,
        &CallerAnchor::from_params(params),
        ResolveScope::Strict,
    )
    .context("dispatch must be run from within a tenex-edge agent session")?;
    let backend_pubkey = match target.backend.as_deref() {
        Some(backend) if backend != state.host => resolve_backend_pubkey(state, backend).await?,
        _ => state
            .backend_pubkey()
            .context("local backend has no management pubkey")?,
    };
    let requested = if p.channels.is_empty() {
        vec![p.workspace.clone()]
    } else {
        p.channels.clone()
    };
    let target_channels = resolve_dispatch_channels(state, &p.workspace, &requested).await?;
    let route_channel = first_shared_channel(state, &caller, &target_channels)?;
    let mut notifications = state.transport.notifications();

    let dispatch_target = crate::fabric::nip29::session_dispatch::DispatchTarget {
        backend_pubkey: backend_pubkey.clone(),
        slug: target.slug.clone(),
        workspace: p.workspace.clone(),
        channels: target_channels.clone(),
    };
    let prose = dispatch_prose(&backend_pubkey, &p.workspace, &target.slug, &requested)?;
    let builder = crate::fabric::nip29::session_dispatch::build_session_dispatch_event(
        &route_channel,
        &dispatch_target,
        &prose,
    )?;
    let keys = state.session_signing_keys(&caller.agent_pubkey)?;
    let signed = state.transport.sign(builder, &keys).await?;
    let dispatch_event_id = signed.id.to_hex();
    state.transport.publish_event_checked(&signed).await?;
    if let Some(op) = crate::fabric::nip29::session_dispatch::parse_session_dispatch(&signed) {
        super::session_dispatch_handler::handle_session_dispatch(state, &signed, op).await;
    }

    let ack = wait_dispatch_ack(&mut notifications, &dispatch_event_id).await?;
    let body = dispatch_message_body(&p.message, &ack.pubkey)?;
    let message_event_id =
        send_dispatch_message(state, &caller, &route_channel, &body, &ack).await?;
    Ok(serde_json::json!({
        "dispatch_event_id": dispatch_event_id,
        "message_event_id": message_event_id,
        "agent": target.slug,
        "workspace": p.workspace,
        "route_channel": route_channel,
        "ack_pubkey": ack.pubkey,
    }))
}

async fn resolve_dispatch_channels(
    state: &Arc<DaemonState>,
    _workspace: &str,
    requested: &[String],
) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for raw in requested {
        let raw = raw.trim();
        if raw.is_empty() {
            anyhow::bail!("dispatch --channel must not be empty");
        }
        let (root, rest) = split_fully_qualified_channel(raw);
        if root.is_empty() || rest == Some("") {
            anyhow::bail!("dispatch --channel {raw:?} is not a valid fully-qualified channel");
        }
        let channel = match rest {
            Some(path) => resolve_channel_path(state, root, path, true).await?,
            None => root.to_string(),
        };
        if !out.iter().any(|seen| seen == &channel) {
            out.push(channel);
        }
    }
    Ok(out)
}

fn split_fully_qualified_channel(raw: &str) -> (&str, Option<&str>) {
    let dot = raw.find('.');
    let slash = raw.find('/');
    match (dot, slash) {
        (Some(d), Some(s)) if d < s => (&raw[..d], Some(&raw[d + 1..])),
        (Some(_), Some(s)) => (&raw[..s], Some(&raw[s + 1..])),
        (Some(d), None) => (&raw[..d], Some(&raw[d + 1..])),
        (None, Some(s)) => (&raw[..s], Some(&raw[s + 1..])),
        (None, None) => (raw, None),
    }
}

fn first_shared_channel(
    state: &Arc<DaemonState>,
    caller: &crate::state::Session,
    target_channels: &[String],
) -> Result<String> {
    let joined = state.with_store(|s| s.list_session_joined_channels(&caller.session_id))?;
    let joined_set: HashSet<&str> = joined.iter().map(|(h, _)| h.as_str()).collect();
    if let Some(ch) = target_channels
        .iter()
        .find(|channel| joined_set.contains(channel.as_str()))
    {
        return Ok(ch.clone());
    }
    let refs = state.with_store(|s| {
        joined
            .iter()
            .map(|(h, _)| super::channel_resolve::channel_reference_for(s, h))
            .collect::<Vec<_>>()
    });
    anyhow::bail!(
        "you need to specify a channel you're active on: {}",
        refs.join(", ")
    )
}

fn dispatch_prose(
    backend_pubkey: &str,
    workspace: &str,
    slug: &str,
    channels: &[String],
) -> Result<String> {
    let npub = PublicKey::from_hex(backend_pubkey)?.to_bech32()?;
    let mut prose = format!("nostr:{npub}: dispatch {workspace}'s {slug}");
    if !channels.is_empty() {
        let label = if channels.len() == 1 {
            "channel"
        } else {
            "channels"
        };
        prose.push_str(&format!(" on {label} {}", channels.join(", ")));
    }
    Ok(prose)
}

fn dispatch_message_body(message: &str, target_pubkey: &str) -> Result<String> {
    let npub = PublicKey::from_hex(target_pubkey)?.to_bech32()?;
    let prefix = format!("nostr:{npub}:");
    if message.trim_start().starts_with(&prefix) {
        return Ok(message.to_string());
    }
    Ok(format!("{prefix} {message}"))
}

struct DispatchAck {
    pubkey: String,
}

async fn wait_dispatch_ack(
    notifications: &mut tokio::sync::broadcast::Receiver<RelayPoolNotification>,
    dispatch_event_id: &str,
) -> Result<DispatchAck> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            anyhow::bail!("dispatched agent did not ACK after 30 seconds");
        }
        let item = tokio::time::timeout(remaining, notifications.recv()).await;
        let event = match item {
            Ok(Ok(RelayPoolNotification::Event { event, .. })) => Some(*event),
            Ok(Ok(RelayPoolNotification::Message {
                message: RelayMessage::Event { event, .. },
                ..
            })) => Some(event.into_owned()),
            Ok(Ok(_)) => None,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => None,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                anyhow::bail!("relay notification stream closed while waiting for dispatch ACK")
            }
            Err(_) => anyhow::bail!("dispatched agent did not ACK after 30 seconds"),
        };
        let Some(event) = event else { continue };
        if crate::fabric::nip29::session_dispatch::dispatch_ack_ref(&event)
            != Some(dispatch_event_id)
        {
            continue;
        }
        return Ok(DispatchAck {
            pubkey: event.pubkey.to_hex(),
        });
    }
}

async fn send_dispatch_message(
    state: &Arc<DaemonState>,
    caller: &crate::state::Session,
    channel: &str,
    message: &str,
    ack: &DispatchAck,
) -> Result<String> {
    let instance = state.session_instance(caller);
    let keys = state.session_signing_keys(&caller.agent_pubkey)?;
    let chat = ChatMessage {
        from: instance.agent_ref(),
        channel: channel.to_string(),
        body: message.to_string(),
        mentioned_pubkeys: vec![ack.pubkey.clone()],
    };
    let published = state
        .provider
        .publish_chat_checked(
            &chat,
            &keys,
            &crate::fabric::provider::chat::OutboundChatRecord {
                channel_h: channel.to_string(),
                body: message.to_string(),
                recipients: vec![OutboundChatRecipient::new(ack.pubkey.clone())],
                created_at: Some(now_secs()),
                direction: "outbound",
            },
        )
        .await?;
    Ok(published.event_id)
}

#[cfg(test)]
mod tests;
