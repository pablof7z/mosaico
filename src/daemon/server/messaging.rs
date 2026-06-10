use super::session::resolve_session;
use super::*;

// ── send_message ─────────────────────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct SendMessageParams {
    recipient: String,
    message: String,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    env_session: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

pub(super) async fn rpc_send_message(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: SendMessageParams =
        serde_json::from_value(params.clone()).context("parsing send_message params")?;
    let rec = resolve_session(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
    )?;
    let id = identity::load_or_create(&config::edge_home(), &rec.agent_slug, now_secs())?;

    let recipient = state.with_store(|s| resolve_recipient(s, &rec.project, &p.recipient))?;

    let mention = Mention {
        from: crate::domain::AgentRef::new(id.pubkey_hex(), rec.agent_slug.clone()),
        to_pubkey: recipient.pubkey.clone(),
        project: recipient.project.clone(),
        body: p.message,
        target_session: recipient.target_session.clone().map(SessionId::from),
        // Stamp the sender's own session so the recipient can reply to it precisely.
        from_session: Some(SessionId::from(rec.session_id.clone())),
    };
    let builder = state.codec.encode(&DomainEvent::Mention(mention.clone()))?;
    // Publish over the shared relay; the returned EventId is the canonical id of
    // the just-signed event.
    let event_id = state.transport.publish_signed(builder, &id.keys).await?;

    // LOCAL DELIVERY (the same-machine fix). When the recipient is an agent this
    // daemon hosts (e.g. a SIBLING claude session sharing the sender's pubkey),
    // delivery must NOT depend on the relay echoing our own published event back
    // to us — relays generally do not re-deliver an event to the connection that
    // published it. Route the mention into the recipient's session inbox(es) here,
    // keyed by the SAME EventId we just published. `route_mention_into` →
    // `enqueue_mention` is idempotent on `(mention_event_id, target_session)`, so
    // if the relay does echo it later, no duplicate is created. `compute_targets`
    // delivers only to the TARGET session (or all of the recipient agent's
    // sessions when untargeted) — never back to the authoring session.
    if state
        .hosted_pubkeys()
        .iter()
        .any(|h| h == &recipient.pubkey)
    {
        let routed = state.with_store(|s| {
            route_mention_into_with_id(s, &recipient.pubkey, &mention, &event_id.to_hex())
        });
        if routed {
            state.mention_notify.notify_waiters();
        }
    }

    Ok(
        serde_json::json!({ "to_pubkey": recipient.pubkey, "target_session": recipient.target_session }),
    )
}

struct ResolvedRecipient {
    pubkey: String,
    target_session: Option<String>,
    project: String,
}

fn resolve_recipient(store: &Store, my_project: &str, target: &str) -> Result<ResolvedRecipient> {
    if let Some((slug, proj)) = target.split_once('@') {
        let pk = store
            .resolve_agent_pubkey(slug, Some(proj))?
            .with_context(|| {
                format!("can't resolve {slug}@{proj} (no presence/profile seen yet)")
            })?;
        return Ok(ResolvedRecipient {
            pubkey: pk,
            target_session: None,
            project: proj.to_string(),
        });
    }
    if target.len() == 64 && target.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(ResolvedRecipient {
            pubkey: target.to_string(),
            target_session: None,
            project: my_project.to_string(),
        });
    }
    if target.len() >= 6 {
        if let Some(ps) = store.find_peer_session_by_prefix(target)? {
            return Ok(ResolvedRecipient {
                pubkey: ps.pubkey,
                target_session: Some(ps.session_id),
                project: ps.project,
            });
        }
        if let Some(s) = store.find_session_by_prefix(target)? {
            return Ok(ResolvedRecipient {
                pubkey: s.agent_pubkey,
                target_session: Some(s.session_id),
                project: s.project,
            });
        }
        // Try matching against hash-based session short codes (from `who` display).
        // This is a fallback for when users copy session codes from `who` output.
        if let Some(found) = find_session_by_hash(store, target)? {
            return Ok(ResolvedRecipient {
                pubkey: found.pubkey,
                target_session: Some(found.session_id),
                project: found.project,
            });
        }
    }
    if let Some(pk) = store.resolve_agent_pubkey(target, Some(my_project))? {
        return Ok(ResolvedRecipient {
            pubkey: pk,
            target_session: None,
            project: my_project.to_string(),
        });
    }
    anyhow::bail!("can't resolve recipient {target:?} (try `tenex-edge who`)")
}

struct SessionMatch {
    pubkey: String,
    session_id: String,
    project: String,
}

/// Try to find a session (peer or own) matching the given hash code.
/// Hash codes are what `who` displays for sessions (6-char hex strings).
fn find_session_by_hash(store: &Store, hash_code: &str) -> Result<Option<SessionMatch>> {
    let target_code = hash_code.to_lowercase();

    // Search peer sessions
    if let Ok(peers) = store.list_peer_sessions(None, 0) {
        for peer in peers {
            if session_short_code(&peer.session_id).to_lowercase() == target_code {
                return Ok(Some(SessionMatch {
                    pubkey: peer.pubkey,
                    session_id: peer.session_id,
                    project: peer.project,
                }));
            }
        }
    }

    // Search own sessions
    if let Ok(sessions) = store.list_my_live_sessions(0) {
        for session in sessions {
            if session_short_code(&session.session_id).to_lowercase() == target_code {
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
