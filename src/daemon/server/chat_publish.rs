use super::chat_write::chat_relay_event;
use super::*;

pub(in crate::daemon::server) struct ChatRecordDraft {
    from_pubkey: String,
    project: String,
    body: String,
}

pub(in crate::daemon::server) async fn publish_chat_checked(
    state: &Arc<DaemonState>,
    chat: &ChatMessage,
    signing: &Keys,
    draft: &ChatRecordDraft,
) -> Result<String> {
    let builder = state
        .provider
        .encode(&DomainEvent::ChatMessage(chat.clone()))?;
    let signed = state.transport.sign(builder, signing).await?;
    let event_id = signed.id.to_hex();
    let created_at = now_secs();

    state.with_store(|s| {
        let _ = s.insert_event(&chat_relay_event(
            &event_id,
            &draft.from_pubkey,
            created_at,
            &draft.project,
            &draft.body,
            None,
        ));
    });

    state.transport.publish_event(&signed).await?;
    Ok(event_id)
}

pub(in crate::daemon::server) fn spawn_chat_publish_retry(
    state: Arc<DaemonState>,
    chat: ChatMessage,
    signing: Keys,
    draft: ChatRecordDraft,
    label: &'static str,
) {
    tokio::spawn(async move {
        let mut last_err = String::new();
        for attempt in 0..60 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
            match publish_chat_checked(&state, &chat, &signing, &draft).await {
                Ok(_) => return,
                Err(e) => last_err = e.to_string(),
            }
        }
        eprintln!("[daemon] {label} kind:9 publish retry exhausted: {last_err}");
    });
}

pub(in crate::daemon::server) fn spawn_retry_drainer(state: Arc<DaemonState>) {
    let queue = state.transport.retry_queue.clone();
    let transport = state.transport.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let due = queue.drain_due();
            for retry in due {
                let id_short = {
                    let h = retry.event.id.to_hex();
                    h[..8.min(h.len())].to_string()
                };
                let kind = retry.event.kind.as_u16();
                if transport.retry_publish(&retry.event).await {
                    eprintln!(
                        "[retry] event {id_short} kind:{kind} accepted on attempt {}",
                        retry.attempt + 1
                    );
                } else {
                    queue.requeue(retry, "relay rejected on retry");
                }
            }
        }
    });
}

/// Publish the agent's completed-turn output as kind:9 chat into the session's
/// room (issue #6). Signed by the agent via the session's `AgentInstance`
/// (selected pubkey + display label), falling back to the base agent key (#5)
/// only when no derived identity is bound. Mirrors `rpc_chat_write`'s publish +
/// local record, minus mention handling.
pub(in crate::daemon::server) async fn publish_agent_reply(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    reply: &str,
) -> Result<()> {
    // Issue #98: the session's authoritative agent-instance identity is the single
    // source for signing key, selected pubkey, and display label — no ad hoc
    // keys_for_session(..).unwrap_or(base) / base-slug pairing at the edge.
    let instance = state.session_instance(rec);
    let base = identity::load_or_create(&config::edge_home(), &instance.base_slug, now_secs())?;
    let signing = instance.signing_keys(&base.keys);
    let from_pubkey = instance.pubkey.clone();
    // Route into the session's CURRENT channel so a `channels switch` moves the
    // agent's turn replies to the new channel without restarting.
    let scope = rec.channel_h.clone();

    let chat = ChatMessage {
        from: instance.agent_ref(),
        project: scope.clone(),
        body: reply.to_string(),
        mentioned_pubkey: None,
    };
    let draft = ChatRecordDraft {
        from_pubkey,
        project: scope,
        body: reply.to_string(),
    };
    let publish = publish_chat_checked(state, &chat, &signing, &draft);
    match tokio::time::timeout(std::time::Duration::from_secs(3), publish).await {
        Ok(Ok(_)) => {}
        Ok(Err(_)) | Err(_) => {
            spawn_chat_publish_retry(state.clone(), chat, signing, draft, "agent reply");
        }
    }
    Ok(())
}

/// Publish a user's prompt as kind:9 chat into the session's room (issue #6).
///
/// The human is speaking, so the event is signed by the OPERATOR key (which is
/// the room's admin) rather than the agent/session key. Fail-open: if no
/// operator key is set the prompt is simply not mirrored — the hook must never
/// block the editor. The session resolves to its room via `resolve_session`,
/// so the prompt lands in the same per-session subgroup the agent posts into.
pub(in crate::daemon::server) async fn rpc_user_prompt(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    use nostr_sdk::prelude::Keys;

    #[derive(serde::Deserialize, Default)]
    struct P {
        #[serde(default)]
        session: Option<String>,
        #[serde(default)]
        env_session: Option<String>,
        #[serde(default)]
        agent: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        prompt: String,
        #[serde(default)]
        subagent_id: Option<String>,
    }
    let p: P = serde_json::from_value(params.clone()).context("parsing user_prompt params")?;
    if p.prompt.trim().is_empty() {
        return Ok(serde_json::json!({ "skipped": "empty prompt" }));
    }

    // Codex fires this same hook — on the same top-level session_id — when it
    // dispatches a prompt to a subagent it spawned (spawn_agent/multi_agent_v1*),
    // not just when the human types. That event carries a `subagent_id`; a
    // genuine keystroke never does. Mirroring it would post the agent's own
    // internal instructions to a subagent into the room as if the human said
    // them (issue #102).
    if p.subagent_id.is_some() {
        return Ok(serde_json::json!({ "skipped": "subagent dispatch, not human input" }));
    }

    // No operator key → nothing to sign with; fail open (session still runs).
    // `userNsec` is the ONLY signer for user prompts — the human is speaking.
    let Some(nsec) = state.cfg.user_nsec() else {
        return Ok(serde_json::json!({ "skipped": "userNsec unset" }));
    };
    let op_keys = Keys::parse(nsec).context("parsing operator key")?;
    let op_pubkey = op_keys.public_key().to_hex();

    let rec = resolve_session(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
        None,
    )?;

    // A daemon-injected fabric envelope (a mention the tmux delivery path pasted
    // into this session's pane) is ALREADY a kind:9 event in the room. The
    // harness re-submits it as a prompt, firing user-prompt-submit; republishing
    // it would echo the message back into the channel (twice, on a publish
    // retry). The echo guard recognizes — and consumes — what we just pasted, so
    // only genuine human keyboard input is mirrored.
    if state.is_injection_echo(&rec.session_id, &p.prompt) {
        return Ok(serde_json::json!({ "skipped": "fabric injection echo" }));
    }

    // Only mirror prompts into a sub-channel (a task/session room). A human start
    // with no resume anchor keeps the session on the top-level project channel;
    // mirroring there would spam the bare project group, so skip it (fail-open).
    // A sub-channel is one whose materialized 39000 carries a parent.
    let is_room = !state
        .with_store(|s| s.is_root_channel(&rec.channel_h))
        .unwrap_or(true);
    if !is_room {
        return Ok(serde_json::json!({ "skipped": "session not in a room" }));
    }

    // Seed the local pre-publish title once, from the real user prompt, so the
    // status pipeline has an immediate title before the runtime transcript read
    // catches up. The title feeds the agent's kind:30315 status only — it never
    // renames the CHANNEL (the channel `name` is set at create/edit alone).
    let session_id = rec.session_id.clone();
    let seeded = state.with_store(|s| {
        let sess = s.get_session(&session_id).ok().flatten()?;
        if !sess.title.trim().is_empty() {
            return None;
        }
        let seed = crate::util::titleize_prompt(&p.prompt);
        if seed.is_empty() {
            return None;
        }
        s.set_session_distill(&session_id, &seed, &sess.activity, now_secs())
            .ok()?;
        Some(seed)
    });
    if seeded.is_some() {
        state.outbox_notify.notify_waiters();
    }

    // Publish into the session's CURRENT channel so the prompt lands where the
    // rest of the session's chat now goes.
    let scope = rec.channel_h.clone();
    let chat = ChatMessage {
        from: crate::domain::AgentRef::new(op_pubkey.clone(), "operator".to_string()),
        project: scope.clone(),
        body: p.prompt.clone(),
        mentioned_pubkey: None,
    };
    let draft = ChatRecordDraft {
        from_pubkey: op_pubkey,
        project: scope.clone(),
        body: p.prompt.clone(),
    };
    // Try once synchronously so local chat history exists when the hook/RPC
    // returns. If relay membership is still converging, fall back to a daemon
    // retry without blocking the editor.
    let publish = publish_chat_checked(state, &chat, &op_keys, &draft);
    match tokio::time::timeout(std::time::Duration::from_secs(3), publish).await {
        Ok(Ok(_)) => {}
        Ok(Err(_)) | Err(_) => {
            spawn_chat_publish_retry(state.clone(), chat, op_keys, draft, "user prompt");
        }
    }

    Ok(serde_json::json!({ "queued": true, "project": scope }))
}
