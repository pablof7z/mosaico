use super::chat_target::resolve_chat_target_provisioning;
use super::resolution::work_root_for;
use super::*;
use crate::fabric::provider::chat::OutboundChatRecord;
use crate::util::CHAT_WRITE_CHAR_LIMIT;
use anyhow::bail;

mod recipient;
#[cfg(test)]
mod tests;

pub(in crate::daemon::server) use recipient::resolve_recipient;

#[derive(serde::Deserialize, Default)]
#[allow(dead_code)]
pub(in crate::daemon::server) struct ChatWriteParams {
    message: String,
    #[serde(default, alias = "env_session")]
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
    current_scope: &str,
    explicit_dest: Option<&str>,
    mention_project: Option<&str>,
) -> String {
    explicit_dest
        .or(mention_project)
        .unwrap_or(current_scope)
        .to_string()
}

pub(in crate::daemon::server) async fn rpc_chat_write(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: ChatWriteParams =
        serde_json::from_value(params.clone()).context("parsing chat_write params")?;
    if long_message_requires_override(&p) {
        bail!(
            "your message is too long; keep it under {CHAT_WRITE_CHAR_LIMIT} characters or pass --long-message"
        );
    }
    let mut anchor = CallerAnchor::from_params(params);
    anchor.group = None;
    let rec = resolve_session(state, &anchor)?;
    let id = identity::load_or_create(&config::edge_home(), &rec.agent_slug, now_secs())?;
    let durable_pubkey = id.pubkey_hex();
    // Routing scope: the channel this session currently publishes into. Caller
    // lookup is independent from destination targeting; `channel` below is a
    // chat destination only, never a session-resolution hint.
    let scope = rec.channel_h.clone();

    let target =
        resolve_chat_target_provisioning(state, &rec, p.channel.as_deref(), "channel send").await?;
    let explicit_dest =
        (target.explicit && target.channel_h != scope).then_some(target.channel_h.clone());
    let body_to_send = match &explicit_dest {
        Some(_) => format!(
            "[from @{} working in #{scope}]: {}",
            rec.agent_slug, p.message
        ),
        None => p.message.clone(),
    };
    // Mention target: the FIRST inline `@<agent-instance-label>` in the body that
    // resolves to a known instance pubkey. A redirect is a plain channel post, not
    // a mention. An unresolvable token is silently treated as no mention — it must
    // never bail or block the chat.
    let mention_token: Option<String> = if explicit_dest.is_some() {
        None
    } else {
        crate::idref::extract_mentions(&p.message)
            .into_iter()
            .next()
    };
    let mention = if let Some(raw) = mention_token {
        match state.with_store(|s| resolve_recipient(s, &scope, &state.host, &raw)) {
            Ok(target) => {
                let same_work_root = state
                    .with_store(|s| work_root_for(s, &scope) == work_root_for(s, &target.project));
                if target.project != scope && !same_work_root {
                    anyhow::bail!(
                        "mention target is in project {:?}, but this chat is for project {:?}",
                        target.project,
                        scope
                    );
                }
                Some((target.pubkey, target.target_session, target.project, raw))
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
        &scope,
        explicit_dest.as_deref(),
        mention.as_ref().map(|(_, _, project, _)| project.as_str()),
    );
    // Local visibility and inbox routing must use the same channel as the signed
    // event's `h` tag. Otherwise relay readback of our own event can disagree
    // with the locally-seeded row and the primary-key de-dupe preserves the wrong
    // scope.
    let deliver_scope = publish_scope.clone();

    // Sign + label from the session's own minted identity (its pubkey + codename).
    let instance = state.session_instance(&rec);
    let chat_signing_keys = state.session_signing_keys(&rec.session_id)?;
    let from_pubkey = instance.pubkey.clone();

    let chat = ChatMessage {
        from: instance.agent_ref(),
        project: publish_scope.clone(),
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
                    "chat_write: listing live sessions for local delivery failed — direct mention may not reach a local inbox/doorbell"
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
                        "chat_write: enqueue_inbox failed — this direct mention may never reach the target's inbox/doorbell"
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
                    "chat_write: recipient session edge upsert failed"
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
        project: deliver_scope.clone(),
        from: from_label,
        from_session: Some(rec.session_id),
        to: mentioned_pubkey
            .as_deref()
            .map(pubkey_short)
            .unwrap_or_else(|| "project-chat".to_string()),
        to_session: mentioned_session.clone(),
        body: body_to_send.chars().take(200).collect(),
    });

    Ok(serde_json::json!({
        "event_id": event_id,
        "project": publish_scope,
        "mentioned_pubkey": mentioned_pubkey,
        "mentioned_session": mentioned_session,
        "mentioned_label": mentioned_label,
    }))
}

fn long_message_requires_override(p: &ChatWriteParams) -> bool {
    !p.long_message && p.message.chars().count() > CHAT_WRITE_CHAR_LIMIT
}

fn handle_mention_resolution_error(raw: &str, e: anyhow::Error) -> Result<()> {
    if e.chain().any(|c| c.is::<rusqlite::Error>()) {
        anyhow::bail!("failed to resolve mention @{raw}: {e:#}");
    }
    Ok(())
}
