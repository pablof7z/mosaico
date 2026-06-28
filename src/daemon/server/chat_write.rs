use super::resolution::work_root_for;
use super::*;
use crate::state::{RelayEvent, Store};

#[derive(serde::Deserialize, Default)]
pub(in crate::daemon::server) struct ChatWriteParams {
    message: String,
    #[serde(default)]
    env_session: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    group: Option<String>,
}

/// Build a verbatim kind:9 chat row for the `relay_events` log from the fields we
/// already know about a freshly-published chat line (the relay rarely echoes a
/// publish back to the same connection, so the log is seeded locally).
pub(in crate::daemon::server) fn chat_relay_event(
    id: &str,
    pubkey: &str,
    created_at: u64,
    channel_h: &str,
    body: &str,
    mentioned: Option<&str>,
) -> RelayEvent {
    let mut tags: Vec<Vec<String>> = vec![vec!["h".to_string(), channel_h.to_string()]];
    if let Some(pk) = mentioned {
        tags.push(vec!["p".to_string(), pk.to_string()]);
    }
    RelayEvent {
        id: id.to_string(),
        kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
        pubkey: pubkey.to_string(),
        created_at,
        channel_h: channel_h.to_string(),
        d_tag: String::new(),
        content: body.to_string(),
        tags_json: serde_json::to_string(&tags).unwrap_or_else(|_| "[]".to_string()),
    }
}

pub(in crate::daemon::server) async fn rpc_chat_write(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: ChatWriteParams =
        serde_json::from_value(params.clone()).context("parsing chat_write params")?;
    let rec = resolve_session(
        state,
        None,
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
        p.group.as_deref(),
    )?;
    let id = identity::load_or_create(&config::edge_home(), &rec.agent_slug, now_secs())?;
    let durable_pubkey = id.pubkey_hex();
    // Routing scope: the channel this session currently publishes into. All chat
    // routing + the wire `h` tag key on this so a switched session's chat lands in
    // the new channel.
    let scope = rec.channel_h.clone();

    // Explicit-destination redirect (issue #47): `chat write --channel test1` from
    // inside a session publishes INTO that channel even though `env_session`
    // resolved the SENDER to its own channel. Resolve the NAME (or literal id) to
    // its opaque `channel_h` within the sender's project scope — erroring if
    // unknown (never a silent literal-`h` send) — and treat it as a redirect only
    // when it differs from the sender's own scope. The daemon injects an
    // authoritative provenance prefix the agent cannot spoof.
    let explicit_dest = match p.group.as_deref().filter(|g| !g.is_empty()) {
        Some(name) => {
            let parent = state.with_store(|s| work_root_for(s, &scope));
            let id =
                super::resolve_channel(state, &parent, name, Some(&rec.agent_slug), false).await?;
            (id != scope).then_some(id)
        }
        None => None,
    };
    let body_to_send = match &explicit_dest {
        Some(_) => format!("[from @{} working in #{scope}]: {}", rec.agent_slug, p.message),
        None => p.message.clone(),
    };
    // Sessions to deliver to + the wire `h`: the explicit destination on a
    // redirect, else the sender's own scope.
    let deliver_scope = explicit_dest.clone().unwrap_or_else(|| scope.clone());

    // Mention target: the FIRST inline `@codename` in the body. A redirect is a
    // plain channel post, not a mention.
    let mention_token: Option<String> = if explicit_dest.is_some() {
        None
    } else {
        crate::idref::extract_mentions(&p.message).into_iter().next()
    };
    let mention = if let Some(raw) = mention_token {
        let target = state.with_store(|s| resolve_recipient(s, &scope, &state.host, &raw))?;
        let Some(session_id) = target.target_session else {
            anyhow::bail!(
                "mention @{raw} must name a concrete session codename from `tenex-edge who`"
            );
        };
        let same_work_root = state.with_store(|s| {
            work_root_for(s, &scope) == work_root_for(s, &target.project)
        });
        if target.project != scope && !same_work_root {
            anyhow::bail!(
                "mention target is in project {:?}, but this chat is for project {:?}",
                target.project,
                scope
            );
        }
        Some((target.pubkey, session_id, target.project))
    } else {
        None
    };
    let mentioned_pubkey = mention.as_ref().map(|(pk, _, _)| pk.clone());
    let mentioned_session = mention.as_ref().map(|(_, sid, _)| sid.clone());
    let publish_scope = explicit_dest.clone().unwrap_or_else(|| {
        mention
            .as_ref()
            .map(|(_, _, project)| project.as_str())
            .unwrap_or(scope.as_str())
            .to_string()
    });

    let chat_signing_keys = state
        .keys_for_session(&rec.session_id)
        .unwrap_or_else(|| id.keys.clone());
    let from_pubkey = chat_signing_keys.public_key().to_hex();

    let chat = ChatMessage {
        from: crate::domain::AgentRef::new(from_pubkey.clone(), rec.agent_slug.clone()),
        project: publish_scope.clone(),
        body: body_to_send.clone(),
        mentioned_pubkey: mentioned_pubkey.clone(),
    };
    let event_id = state
        .provider
        .publish_checked(&DomainEvent::ChatMessage(chat), &chat_signing_keys)
        .await?;
    let event_id = event_id.to_hex();
    let created_at = now_secs();

    // Local live delivery: relays often don't echo an event back to the same
    // connection that published it. Seed the verbatim log and park inbox rows for
    // sessions already alive in the same routing scope.
    let routed = state.with_store(|s| {
        let _ = s.insert_event(&chat_relay_event(
            &event_id,
            &from_pubkey,
            created_at,
            &deliver_scope,
            &body_to_send,
            mentioned_pubkey.as_deref(),
        ));
        let mut routed = false;
        for target in s.list_alive_sessions().unwrap_or_default() {
            let is_direct_target = mentioned_session.as_deref() == Some(target.session_id.as_str());
            if !is_direct_target && target.channel_h != deliver_scope {
                continue;
            }
            if target.created_at > created_at {
                continue;
            }
            // Skip sender's own sessions by pubkey.
            if target.agent_pubkey == durable_pubkey || target.agent_pubkey == from_pubkey {
                continue;
            }
            let row_channel = if is_direct_target {
                target.channel_h.clone()
            } else {
                deliver_scope.clone()
            };
            if s.enqueue_inbox(
                &event_id,
                &target.session_id,
                &from_pubkey,
                &row_channel,
                &body_to_send,
                created_at,
            )
            .unwrap_or(false)
            {
                routed = true;
            }
        }
        routed
    });
    if routed {
        crate::tmux::ring_doorbells(state.clone());
    }

    state.emit_tail(TailEvent::Msg {
        ts: created_at,
        project: deliver_scope.clone(),
        from: rec.agent_slug,
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
    }))
}

pub(in crate::daemon::server) struct ResolvedRecipient {
    pubkey: String,
    target_session: Option<String>,
    project: String,
}

/// Resolve a recipient/identifier to a wire pubkey under the CANONICAL scheme:
///   - `agent@host`  → the durable agent on that machine (host always slugified;
///     `@` NEVER means project). The message still goes to `my_project`.
///   - 64-hex / npub → raw pubkey.
///   - a session     → by canonical id, harness alias, id prefix, or codename.
///   - a bare slug   → that agent on the LOCAL host (`slug@<local_host>`).
///
/// Sessions are local-only in the new model (session ids never travel the wire),
/// so session-prefix / codename matching searches the local `sessions` table; a
/// remote agent is addressed only by `agent@host` or pubkey.
pub(in crate::daemon::server) fn resolve_recipient(
    store: &Store,
    my_project: &str,
    local_host: &str,
    target: &str,
) -> Result<ResolvedRecipient> {
    use crate::idref::{parse_ref, Ref};

    let session_recipient =
        |store: &Store, session_id: String, fallback_pk: String, project: String| {
            ResolvedRecipient {
                pubkey: store
                    .get_session(&session_id)
                    .ok()
                    .flatten()
                    .map(|s| s.agent_pubkey)
                    .unwrap_or(fallback_pk),
                target_session: Some(session_id),
                project,
            }
        };

    match parse_ref(target) {
        Ref::Agent { slug, host } => {
            let pk = store
                .resolve_agent_pubkey(&slug, &host)?
                .with_context(|| format!("can't resolve {slug}@{host} (no profile seen yet — try `tenex-edge who`)"))?;
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
                return Ok(session_recipient(store, s.session_id, s.agent_pubkey, s.channel_h));
            }
            // 2. Local session id prefix.
            if tok.len() >= 6 {
                if let Some(s) = store
                    .list_alive_sessions()
                    .unwrap_or_default()
                    .into_iter()
                    .find(|s| s.session_id.starts_with(&tok))
                {
                    return Ok(session_recipient(store, s.session_id, s.agent_pubkey, s.channel_h));
                }
            }
            // 3. Session codename (e.g. `bravo4217` from `who`).
            if let Some(found) = find_session_by_codename(store, &tok)? {
                return Ok(session_recipient(
                    store,
                    found.session_id,
                    found.pubkey,
                    found.project,
                ));
            }
            // 4. Bare agent slug → that agent on the LOCAL host.
            if let Some(pk) =
                store.resolve_agent_pubkey(&tok, &crate::util::slugify_host(local_host))?
            {
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

pub(in crate::daemon::server) struct SessionMatch {
    pubkey: String,
    session_id: String,
    project: String,
}

/// Try to find a LOCAL session matching the given codename (what `who` displays,
/// e.g. `bravo4217`). Remote agents have no local session and are addressed by
/// `agent@host`/pubkey instead.
pub(in crate::daemon::server) fn find_session_by_codename(
    store: &Store,
    codename: &str,
) -> Result<Option<SessionMatch>> {
    let target_code = codename.to_lowercase();
    for session in store.list_alive_sessions().unwrap_or_default() {
        if session_codename(&session.session_id).to_lowercase() == target_code {
            return Ok(Some(SessionMatch {
                pubkey: session.agent_pubkey,
                session_id: session.session_id,
                project: session.channel_h,
            }));
        }
    }
    Ok(None)
}
