use super::*;
use anyhow::Context;
use nostr::{PublicKey, ToBech32};
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
    .context("dispatch must be run from within a mosaico agent session")?;
    let backend_pubkey = match target.backend.as_deref() {
        Some(backend) if backend != state.host => resolve_backend_pubkey(state, backend).await?,
        _ => state
            .backend_pubkey()
            .context("local backend has no management pubkey")?,
    };
    let requested = if p.channels.is_empty() {
        vec![crate::channel_ref::format_channel_ref(&p.workspace, &[])]
    } else {
        p.channels.clone()
    };
    let target_channels = resolve_dispatch_channels(state, &p.workspace, &requested).await?;
    let route_channel = first_shared_channel(state, &caller, &target_channels)?;
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
    let keys = state.session_signing_keys(&caller.pubkey)?;
    let signed = state.nmp.sign_event(builder, &keys).await?;
    let dispatch_event_id = signed.id.to_hex();
    let ack_events = state.nmp.observe(&dispatch_ack_query(&dispatch_event_id))?;
    state.nmp.publish_group_event(&signed, true).await?;
    if let Some(op) = crate::fabric::nip29::session_dispatch::parse_session_dispatch(&signed) {
        super::session_dispatch_handler::handle_session_dispatch(state, &signed, op).await;
    }

    let ack = wait_dispatch_ack(ack_events, dispatch_event_id.clone()).await?;
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
        let Some((root, rest)) = split_fully_qualified_channel(raw) else {
            anyhow::bail!("dispatch --channel {raw:?} is not a valid fully-qualified channel");
        };
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

fn split_fully_qualified_channel(raw: &str) -> Option<(&str, Option<&str>)> {
    let path = raw.strip_prefix('/')?;
    if path.is_empty() || path.ends_with('/') || path.contains("//") || path.contains('.') {
        return None;
    }
    Some(match path.split_once('/') {
        Some((root, rest)) => (root, Some(rest)),
        None => (path, None),
    })
}

fn first_shared_channel(
    state: &Arc<DaemonState>,
    caller: &crate::state::Session,
    target_channels: &[String],
) -> Result<String> {
    let joined = state.with_store(|s| s.list_session_routes(&caller.pubkey))?;
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
            .collect::<Result<Vec<_>>>()
    })?;
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

fn dispatch_ack_query(dispatch_event_id: &str) -> crate::reconcile::SubscriptionQuery {
    crate::reconcile::SubscriptionQuery {
        kinds: std::collections::BTreeSet::from([crate::fabric::nip29::wire::KIND_STATUS]),
        authors: std::collections::BTreeSet::new(),
        tag: Some(('e', dispatch_event_id.to_string())),
    }
}

async fn wait_dispatch_ack(
    events: nmp::Subscription,
    dispatch_event_id: String,
) -> Result<DispatchAck> {
    tokio::task::spawn_blocking(move || wait_dispatch_ack_blocking(events, &dispatch_event_id))
        .await
        .context("joining dispatch ACK observation")?
}

fn wait_dispatch_ack_blocking(
    events: nmp::Subscription,
    dispatch_event_id: &str,
) -> Result<DispatchAck> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            anyhow::bail!("dispatched agent did not ACK after 30 seconds");
        }
        let frame = match events.recv_timeout(remaining) {
            Ok(frame) => frame,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                anyhow::bail!("dispatched agent did not ACK after 30 seconds")
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                anyhow::bail!("relay notification stream closed while waiting for dispatch ACK")
            }
        };
        for event in frame.deltas.iter().filter_map(|delta| delta.event()) {
            if crate::fabric::nip29::session_dispatch::dispatch_ack_ref(event)
                == Some(dispatch_event_id)
            {
                return Ok(DispatchAck {
                    pubkey: event.pubkey.to_hex(),
                });
            }
        }
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
    let keys = state.session_signing_keys(&caller.pubkey)?;
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
                direction: "outbound",
            },
        )
        .await?;
    Ok(published.event_id)
}

#[cfg(test)]
mod tests;
