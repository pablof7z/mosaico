use super::chat_target::resolve_chat_target;
use super::resolution::work_root_for;
use super::*;
use crate::fabric::provider::chat::OutboundChatRecord;
use crate::state::Store;
use crate::util::CHAT_WRITE_CHAR_LIMIT;
use anyhow::bail;

#[cfg(test)]
mod tests;

#[derive(serde::Deserialize, Default)]
#[allow(dead_code)]
pub(in crate::daemon::server) struct ChatWriteParams {
    message: String,
    #[serde(default, alias = "env_session")]
    harness_session: Option<String>,
    #[serde(default)]
    tmux_pane: Option<String>,
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

    let target = resolve_chat_target(state, &rec, p.channel.as_deref(), "chat write")?;
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

    // Issue #98: sign + label from the session's authoritative agent-instance
    // identity (selected pubkey + display label), never base-key fallback.
    let instance = state.session_instance(&rec);
    let base = identity::load_or_create(&config::edge_home(), &instance.base_slug, now_secs())?;
    let chat_signing_keys = instance.signing_keys(&base.keys);
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
        crate::tmux::ring_doorbells(state.clone());
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

pub(in crate::daemon::server) struct ResolvedRecipient {
    pubkey: String,
    target_session: Option<String>,
    project: String,
}

/// Resolve a recipient/identifier to a wire pubkey under the CANONICAL scheme:
///   - `agent@backend-label` → the durable agent on that backend (`@` NEVER
///     means project). The message still goes to `my_project`.
///   - 64-hex / npub → raw pubkey.
///   - a session     → by canonical id, harness alias, or id prefix (correlation
///     handles only; a session id is never a chat-target identity).
///   - a bare agent-instance label → that instance on the LOCAL host
///     (`label@<local_host>`), reverse-resolved to its selected pubkey.
///
/// Sessions are local-only in the new model (session ids never travel the wire),
/// so session-prefix matching searches the local `sessions` table; a remote agent
/// is addressed only by `agent@backend-label` or pubkey.
pub(in crate::daemon::server) fn resolve_recipient(
    store: &Store,
    my_project: &str,
    local_host: &str,
    target: &str,
) -> Result<ResolvedRecipient> {
    use crate::idref::{parse_ref, Ref};

    let session_recipient =
        |store: &Store, session_id: String, fallback_pk: String, project: String| {
            let pubkey = store
                .instance_identity_for_session(&session_id)
                .ok()
                .flatten()
                .map(|i| i.pubkey)
                .or_else(|| {
                    store
                        .get_session(&session_id)
                        .ok()
                        .flatten()
                        .map(|s| s.agent_pubkey)
                })
                .unwrap_or(fallback_pk);
            ResolvedRecipient {
                pubkey,
                target_session: Some(session_id),
                project,
            }
        };

    match parse_ref(target) {
        Ref::Agent { slug, host } => {
            let pk = store.resolve_agent_pubkey(&slug, &host)?.with_context(|| {
                format!("can't resolve {slug}@{host} (no profile seen yet — try `tenex-edge who`)")
            })?;
            Ok(ResolvedRecipient {
                pubkey: pk,
                target_session: None,
                project: my_project.to_string(),
            })
        }
        Ref::Pubkey(raw) => {
            let pubkey = nostr_sdk::prelude::PublicKey::parse(&raw)
                .map(|pk| pk.to_hex())
                .unwrap_or(raw);
            Ok(ResolvedRecipient {
                pubkey,
                target_session: None,
                project: my_project.to_string(),
            })
        }
        Ref::Token(tok) => {
            // 1. Exact canonical id or harness alias.
            if let Some(s) = store.get_session(&tok)? {
                return Ok(session_recipient(
                    store,
                    s.session_id,
                    s.agent_pubkey,
                    s.channel_h,
                ));
            }
            // 2. Local session id prefix. A store error here must NOT collapse into
            // "no such recipient" — propagate it so a DB failure is loud, not a
            // silent unknown-mention.
            if tok.len() >= 6 {
                if let Some(s) = store
                    .list_alive_sessions()
                    .context("resolve_recipient: listing live sessions for id-prefix match")?
                    .into_iter()
                    .find(|s| s.session_id.starts_with(&tok))
                {
                    return Ok(session_recipient(
                        store,
                        s.session_id,
                        s.agent_pubkey,
                        s.channel_h,
                    ));
                }
            }
            // 3. Live local agent-instance label from `who` (`haiku`, `haiku1`, ...).
            if let Some(found) = find_session_by_agent_label(store, my_project, &tok)? {
                return Ok(session_recipient(
                    store,
                    found.session_id,
                    found.pubkey,
                    found.project,
                ));
            }
            // 4. Bare agent-instance label → that instance on the LOCAL host
            //    (profile fallback for remote/snapshotted peers).
            if let Some(pk) = store.resolve_agent_pubkey(&tok, local_host.trim())? {
                return Ok(ResolvedRecipient {
                    pubkey: pk,
                    target_session: None,
                    project: my_project.to_string(),
                });
            }
            anyhow::bail!("can't resolve recipient {target:?} (try `tenex-edge who`)")
        }
    }
}

#[derive(Clone)]
struct SessionMatch {
    pubkey: String,
    session_id: String,
    project: String,
}

fn find_session_by_agent_label(
    store: &Store,
    my_project: &str,
    label: &str,
) -> Result<Option<SessionMatch>> {
    let wanted = label.trim().to_ascii_lowercase();
    if wanted.is_empty() {
        return Ok(None);
    }

    let my_root = work_root_for(store, my_project);
    let mut same_scope = Vec::new();
    let mut same_root = Vec::new();
    let mut global = Vec::new();

    for session in store
        .list_alive_sessions()
        .context("find_session_by_agent_label: listing live sessions")?
    {
        let instance = store
            .instance_identity_for_session(&session.session_id)
            .ok()
            .flatten()
            .unwrap_or_else(|| {
                crate::identity::AgentInstance::base(
                    session.agent_slug.clone(),
                    session.agent_pubkey.clone(),
                )
            });
        if instance.display_slug().to_ascii_lowercase() != wanted {
            continue;
        }
        let joined_current = store
            .is_session_joined_channel(&session.session_id, my_project)
            .unwrap_or(session.channel_h == my_project);
        let project = if joined_current {
            my_project.to_string()
        } else {
            session.channel_h.clone()
        };
        let matched = SessionMatch {
            pubkey: instance.pubkey,
            session_id: session.session_id.clone(),
            project,
        };
        if joined_current {
            same_scope.push(matched.clone());
        } else if work_root_for(store, &session.channel_h) == my_root {
            same_root.push(matched.clone());
        }
        global.push(matched);
    }

    if let Some(matched) = choose_unique_session_label_match(label, "current channel", same_scope)?
    {
        return Ok(Some(matched));
    }
    if let Some(matched) = choose_unique_session_label_match(label, "current project", same_root)? {
        return Ok(Some(matched));
    }
    choose_unique_session_label_match(label, "all channels", global)
}

fn choose_unique_session_label_match(
    label: &str,
    scope: &str,
    mut matches: Vec<SessionMatch>,
) -> Result<Option<SessionMatch>> {
    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.pop()),
        _ => anyhow::bail!(
            "agent label @{label} matches multiple live sessions in {scope}; run `tenex-edge who`"
        ),
    }
}
