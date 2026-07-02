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

    #[allow(dead_code)]
    #[derive(serde::Deserialize, Default)]
    struct P {
        #[serde(default)]
        session: Option<String>,
        #[serde(default, alias = "env_session")]
        harness_session: Option<String>,
        #[serde(default)]
        tmux_pane: Option<String>,
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

    let rec = resolve_session(state, &CallerAnchor::from_params(params))?;

    // A daemon-injected fabric envelope (a mention the tmux delivery path pasted
    // into this session's pane) is ALREADY a kind:9 event in the room. The
    // harness re-submits it as a prompt, firing user-prompt-submit; republishing
    // it would echo the message back into the channel. The inbox ledger records
    // the exact delivered event ids that were pasted and consumes that record
    // here, so delayed hooks do not rely on a short-lived text hash.
    if consume_injected_prompt_echo(state, &rec.session_id, &p.prompt)? {
        return Ok(serde_json::json!({ "skipped": "fabric injection echo" }));
    }

    // The harness itself can resume a session with synthetic control content
    // (a background task-completion notification, a system reminder, ...)
    // delivered through the same user-prompt-submit hook as real typed input.
    // That is harness plumbing, not the human speaking — mirroring it would
    // post raw `<task-notification>...</task-notification>`-shaped blobs into
    // the channel verbatim. Skip anything that is, start to finish, one such
    // wrapper element.
    if crate::util::is_harness_envelope(&p.prompt) {
        return Ok(serde_json::json!({ "skipped": "harness envelope" }));
    }

    // Only mirror prompts into a sub-channel (a task/session room). A human start
    // with no resume anchor keeps the session on the top-level project channel;
    // mirroring there would spam the bare project group, so skip it (fail-open).
    // A sub-channel is known channel ancestry whose top-level project root is a
    // different channel; unknown ancestry is not a publishable room.
    let is_room = state.with_store(|s| matches!(s.is_subchannel(&rec.channel_h), Ok(true)));
    if !is_room {
        return Ok(serde_json::json!({ "skipped": "session not in a room" }));
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

fn consume_injected_prompt_echo(
    state: &Arc<DaemonState>,
    session_id: &str,
    prompt: &str,
) -> Result<bool> {
    let whitelisted = state.whitelisted_pubkeys().to_vec();
    let now = now_secs();
    state.with_store(|s| {
        consume_injected_prompt_echo_in_store(s, session_id, prompt, &whitelisted, now)
    })
}

fn consume_injected_prompt_echo_in_store(
    store: &crate::state::Store,
    session_id: &str,
    prompt: &str,
    whitelisted: &[String],
    now: u64,
) -> Result<bool> {
    let mut rows = store.injected_for_session(session_id)?;
    rows.sort_by_key(|r| (r.delivered_at, r.created_at, r.event_id.clone()));
    let want = prompt.trim();
    let mut start = 0;
    while start < rows.len() {
        let delivered_at = rows[start].delivered_at;
        let mut end = start + 1;
        while end < rows.len() && rows[end].delivered_at == delivered_at {
            end += 1;
        }
        let group = &rows[start..end];
        if let Some(rendered) =
            crate::injection::render_tmux_mention(store, group, whitelisted, now)
        {
            if rendered.trim() == want {
                let ids = group
                    .iter()
                    .map(|row| row.event_id.clone())
                    .collect::<Vec<_>>();
                store.consume_injected_echo(&ids, session_id)?;
                return Ok(true);
            }
        }
        start = end;
    }
    Ok(false)
}

#[cfg(test)]
#[path = "chat_publish/tests.rs"]
mod tests;
