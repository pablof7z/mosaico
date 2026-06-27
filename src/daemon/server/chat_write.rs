use super::*;

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
    // Routing scope: the NIP-29 group this session currently publishes into —
    // its `channel` when set (a `channels switch` moved it to a subgroup), else
    // its per-session room (`project`). All chat routing + the wire `h` tag key
    // on this so a switched session's chat lands in the new room, not the old
    // one it minted at spawn.
    let scope = rec.route_scope().to_string();

    // Explicit-destination redirect (issue #47): `chat write --channel #dst` from
    // inside a session publishes INTO #dst even though `env_session` resolved the
    // SENDER to its own room. `resolve_session` uses `group` only to LOCATE the
    // sender (env_session wins); here it becomes the publish TARGET when it
    // differs from the sender's own scope. The canonical case is the
    // channel-switch redirect: a blocked instance messages the instance already
    // active in #dst — accepted because it signs as the same ordinal pubkey,
    // which IS a member there. The daemon injects an authoritative provenance
    // prefix the agent cannot spoof (it is added here, not taken from content).
    let explicit_dest = p
        .group
        .as_deref()
        .filter(|g| !g.is_empty() && *g != scope.as_str())
        .map(str::to_string);
    // Body actually published/recorded: provenance-prefixed on a redirect.
    let body_to_send = match &explicit_dest {
        Some(_) => {
            let label = state
                .with_store(|s| s.identity_route_for_session(&rec.session_id))
                .map(|r| r.label)
                .unwrap_or_else(|| rec.agent_slug.clone());
            format!("[from @{label} working in #{scope}]: {}", p.message)
        }
        None => p.message.clone(),
    };
    // Sessions to deliver to + the wire `h`: the explicit destination on a
    // redirect, else the sender's own scope.
    let deliver_scope = explicit_dest.clone().unwrap_or_else(|| scope.clone());

    // Mention target: the FIRST inline `@codename` found in the message body,
    // so `chat write "hey @bravo4217"` highlights that session. Only codename-
    // shaped tokens (`<nato-word><digits>`) are recognized — `@` means host in
    // every other tenex-edge identifier, so `@codex` / `@codex@laptop` are NOT
    // mentions. See `idref::extract_mentions`.
    // A redirect (`--channel #dst`) is a plain channel post, not a mention — skip
    // mention resolution so it can't rewrite the publish scope or fail
    // cross-room validation.
    let mention_token: Option<String> = if explicit_dest.is_some() {
        None
    } else {
        crate::idref::extract_mentions(&p.message)
            .into_iter()
            .next()
    };
    let mention = if let Some(raw) = mention_token {
        let target = state.with_store(|s| resolve_recipient(s, &scope, &state.host, &raw))?;
        let Some(session_id) = target.target_session else {
            anyhow::bail!(
                "mention @{raw} must name a concrete session codename from `tenex-edge who`"
            );
        };
        let same_work_root = state.with_store(|s| {
            let source_root = s.work_root_for_scope(&scope)?;
            let target_root = s.work_root_for_scope(&target.project)?;
            Ok::<bool, anyhow::Error>(source_root == target_root)
        })?;
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
    // connection that published it, and chat intentionally does not catch up old
    // history. Route now to sessions already alive in the same routing scope
    // (channel when set, else the per-session room) so a `channels switch` is
    // reflected immediately, not only once the relay echoes back.
    let routed = state.with_store(|s| {
        let _ = s.record_chat(&ChatLogRow {
            chat_event_id: event_id.clone(),
            from_pubkey: from_pubkey.clone(),
            from_slug: rec.agent_slug.clone(),
            host: state.host.clone(),
            project: deliver_scope.clone(),
            body: body_to_send.clone(),
            created_at,
            from_session: rec.session_id.clone(),
            mentioned_session: mentioned_session.clone().unwrap_or_default(),
        });
        let mut routed = false;
        for target in s.list_alive_sessions().unwrap_or_default() {
            let is_direct_target = mentioned_session.as_deref() == Some(target.session_id.as_str());
            if !is_direct_target && target.route_scope() != deliver_scope {
                continue;
            }
            if target.created_at > created_at {
                continue;
            }
            // Skip sender's own sessions by pubkey.
            if target.agent_pubkey == durable_pubkey || target.agent_pubkey == from_pubkey {
                continue;
            }
            // Preserve local mention routing: if the resolved mention targets
            // this session, mark it as a direct mention in the inbox row.
            let row_mentioned = if is_direct_target {
                target.session_id.clone()
            } else {
                String::new()
            };
            let row_project = if is_direct_target {
                target.route_scope().to_string()
            } else {
                deliver_scope.clone()
            };
            let row = ChatInboxRow {
                chat_event_id: event_id.clone(),
                target_session: target.session_id,
                from_pubkey: from_pubkey.clone(),
                from_slug: rec.agent_slug.clone(),
                project: row_project,
                body: body_to_send.clone(),
                created_at,
                from_session: rec.session_id.clone(),
                mentioned_session: row_mentioned,
            };
            if s.enqueue_chat(&row).unwrap_or(false) {
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
pub(in crate::daemon::server) fn resolve_recipient(
    store: &Store,
    my_project: &str,
    local_host: &str,
    target: &str,
) -> Result<ResolvedRecipient> {
    use crate::idref::{parse_ref, Ref};

    // A session target scopes local delivery and p-tags the session's selected
    // fabric identity: durable by default, transient for collision-fallback
    // duplicates.
    let session_recipient =
        |store: &Store, session_id: String, fallback_pk: String, project: String| {
            ResolvedRecipient {
                pubkey: store
                    .session_pubkey_for_session(&session_id)
                    .unwrap_or(fallback_pk),
                target_session: Some(session_id),
                project,
            }
        };

    match parse_ref(target) {
        // `agent@host` — durable agent on a specific machine.
        Ref::Agent { slug, host } => {
            let pk = store
                .pubkey_for_agent_on_host(&slug, &host)?
                .with_context(|| format!("can't resolve {slug}@{host} (no presence/profile seen yet — try `tenex-edge who`)"))?;
            Ok(ResolvedRecipient {
                pubkey: pk,
                target_session: None,
                project: my_project.to_string(),
            })
        }
        // 64-hex or npub.
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
        // A session (id / alias / prefix / codename) OR a bare agent slug.
        Ref::Token(tok) => {
            // 1. Exact canonical id or harness alias.
            if let Some(s) = store.get_session(&tok)? {
                return Ok(session_recipient(
                    store,
                    s.session_id,
                    s.agent_pubkey,
                    s.project,
                ));
            }
            // 2. Session id prefix (peer presence, then own sessions).
            if tok.len() >= 6 {
                if let Some(ps) = store
                    .peer_session_snapshots(None, 0)
                    .unwrap_or_default()
                    .into_iter()
                    .find(|ps| ps.session_id.as_str().starts_with(&tok))
                {
                    return Ok(session_recipient(
                        store,
                        ps.session_id.as_str().to_string(),
                        ps.agent_pubkey,
                        ps.project,
                    ));
                }
                if let Some(s) = store.find_session_by_prefix(&tok)? {
                    return Ok(session_recipient(
                        store,
                        s.session_id,
                        s.agent_pubkey,
                        s.project,
                    ));
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
                store.pubkey_for_agent_on_host(&tok, &crate::util::slugify_host(local_host))?
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

/// Try to find a session (peer or own) matching the given codename.
/// Codenames are what `who` displays for sessions (e.g. `bravo4217`).
pub(in crate::daemon::server) fn find_session_by_codename(
    store: &Store,
    codename: &str,
) -> Result<Option<SessionMatch>> {
    let target_code = codename.to_lowercase();

    // Search peer sessions. Production peer presence lives in `peer_session_state`
    // (written by `record_peer_status`), surfaced via `peer_session_snapshots`;
    // the `peer_sessions` table is only populated by tests. The snapshot's
    // `agent_pubkey` is the peer's SESSION pubkey (peer status is session-signed),
    // which is exactly the wire address we want to p-tag.
    if let Ok(peers) = store.peer_session_snapshots(None, 0) {
        for peer in peers {
            if session_codename(peer.session_id.as_str()).to_lowercase() == target_code {
                return Ok(Some(SessionMatch {
                    pubkey: peer.agent_pubkey,
                    session_id: peer.session_id.as_str().to_string(),
                    project: peer.project,
                }));
            }
        }
    }

    // Search own sessions
    if let Ok(sessions) = store.list_my_live_sessions(0) {
        for session in sessions {
            if session_codename(&session.session_id).to_lowercase() == target_code {
                return Ok(Some(SessionMatch {
                    pubkey: session.agent_pubkey,
                    session_id: session.session_id,
                    project: session.project,
                }));
            }
        }
    }

    Ok(None)
}
