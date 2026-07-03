use super::*;

#[derive(serde::Deserialize, Default)]
pub(in crate::daemon::server) struct TurnStartParams {
    session: String,
    #[serde(default)]
    transcript: Option<String>,
}

pub(in crate::daemon::server) async fn rpc_turn_start(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: TurnStartParams =
        serde_json::from_value(params.clone()).context("parsing turn_start params")?;
    if p.session.is_empty() {
        return Ok(serde_json::json!({
            "context": serde_json::Value::Null,
            "audit": {
                "kind": "turn_start",
                "skipped": "empty-session-id",
                "output": { "emitted": false, "bytes": 0, "text": null },
            },
        }));
    }
    // Hooks speak the harness id; every mutator below resolves it to the canonical
    // session id internally, so passing the raw alias is correct. Read the previous
    // turn_started_at BEFORE opening the turn for audit/debug context; durable
    // snapshot-vs-delta gating lives on the session's seen_cursor.
    let prev_started = state
        .with_store(|s| s.get_session(&p.session).ok().flatten())
        .map(|r| r.turn_started_at)
        .unwrap_or(0);

    let now = now_secs();
    state.with_store(|s| {
        // Canonical transition: working=1, turn_started_at=now (alias-resolving).
        if let Err(e) = s.set_working(&p.session, true, now) {
            tracing::error!(session = %p.session, error = %e, "turn_start: set_working(true) failed — session may not show as working");
        }
        if let Some(path) = p.transcript.as_deref().filter(|x| !x.is_empty()) {
            if let Err(e) = s.set_session_transcript(&p.session, path) {
                tracing::error!(session = %p.session, error = %e, "turn_start: set_session_transcript failed — distill will lack a transcript path");
            }
        }
    });
    state.outbox_notify.notify_waiters();

    let rec = match state.with_store(|s| s.get_session(&p.session).ok().flatten()) {
        Some(r) => r,
        None => {
            return Ok(serde_json::json!({
                "context": serde_json::Value::Null,
                "audit": {
                    "kind": "turn_start",
                    "skipped": "session-not-found",
                    "input_session": p.session,
                    "prev_turn_started_at": prev_started,
                    "output": { "emitted": false, "bytes": 0, "text": null },
                },
            }));
        }
    };

    let instance = state.session_instance(&rec);
    let agent_label = instance.display_slug();

    // Emit Turn{working} for the live tail feed, keyed on the routing scope.
    state.emit_tail(TailEvent::Turn {
        ts: now,
        project: rec.channel_h.clone(),
        agent: agent_label,
        session: rec.session_id.clone(),
        state: "working".into(),
        elapsed_s: None,
    });

    // Warm the kind:0 profile cache for identities the synchronous context renderer
    // needs to name: pending inbound senders + body mentions + channel members.
    let to_warm: Vec<String> = state.with_store(|s| {
        let mut v: Vec<String> = Vec::new();
        for r in s
            .peek_pending_for_session(&rec.session_id)
            .unwrap_or_default()
        {
            v.push(r.from_pubkey);
            v.extend(crate::profile::body_mention_pubkeys(&r.body));
        }
        let channels = s
            .list_session_joined_channels(&rec.session_id)
            .unwrap_or_else(|_| vec![(rec.channel_h.clone(), rec.created_at)]);
        for (channel_h, _) in channels {
            for m in s.list_channel_members(&channel_h).unwrap_or_default() {
                v.push(m.pubkey);
            }
        }
        v.sort();
        v.dedup();
        v
    });
    crate::profile::warm(state, &to_warm).await;

    // Assemble via the shared turn-context module so daemon and hook tests cannot drift.
    let backend_pubkey = state.backend_pubkey.clone().unwrap_or_default();
    let base = crate::turn_context::assemble_turn_start_context(
        &state.store,
        &rec,
        &backend_pubkey,
        &state.host,
        prev_started,
    );
    let audit = crate::turn_context::turn_start_audit(
        &state.store,
        &rec,
        prev_started,
        now,
        base.as_deref(),
    );
    let context = base
        .map(serde_json::Value::String)
        .unwrap_or(serde_json::Value::Null);
    Ok(serde_json::json!({ "context": context, "audit": audit }))
}

pub(in crate::daemon::server) fn rpc_turn_check(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let rec = resolve_session(state, &CallerAnchor::from_params(params))?;
    let now = now_secs();
    // The sibling-session delta is rendered from the awareness high-water mark
    // (`seen_cursor`). We advance the cursor atomically — only the first of any
    // concurrent PostToolUse hooks wins the CAS; the rest get delta_since=None
    // and emit nothing, preventing duplicate injections from parallel tool calls.
    let mut cursor_advanced = false;
    let delta_since = if rec.working {
        let old = rec.seen_cursor;
        let won = state
            .with_store(|s| s.try_advance_seen_cursor(&rec.session_id, old, now))
            .unwrap_or(false);
        cursor_advanced = won;
        won.then_some(old)
    } else {
        None
    };
    let base = crate::turn_context::assemble_turn_check_context(
        &state.store,
        &rec,
        &state.host,
        delta_since,
        now,
    );
    let audit = crate::turn_context::turn_check_audit(
        &state.store,
        &rec,
        delta_since,
        cursor_advanced,
        now,
        base.as_deref(),
    );
    let context = base
        .map(serde_json::Value::String)
        .unwrap_or(serde_json::Value::Null);
    Ok(serde_json::json!({ "context": context, "audit": audit }))
}

#[derive(serde::Deserialize)]
pub(in crate::daemon::server) struct TurnEndParams {
    session: String,
}

pub(in crate::daemon::server) async fn rpc_turn_end(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: TurnEndParams =
        serde_json::from_value(params.clone()).context("parsing turn_end params")?;
    if p.session.is_empty() {
        return Ok(serde_json::json!({ "ok": true }));
    }
    // Read working/turn_started_at BEFORE closing the turn so we can compute
    // elapsed (alias-resolving read).
    let pre = state.with_store(|s| s.get_session(&p.session).ok().flatten());
    let (was_working, turn_started_at) = pre
        .as_ref()
        .map(|r| (r.working, r.turn_started_at))
        .unwrap_or((false, 0));
    state.with_store(|s| {
        // Canonical transition: working=0 (alias-resolving). The TITLE is retained.
        if let Err(e) = s.set_working(&p.session, false, 0) {
            tracing::error!(session = %p.session, error = %e, "turn_end: set_working(false) failed — session may remain stuck as working");
        }
    });
    state.outbox_notify.notify_waiters();

    let rec = state.with_store(|s| s.get_session(&p.session).ok().flatten());

    if was_working {
        let now = now_secs();
        let elapsed_s = (turn_started_at > 0).then(|| now.saturating_sub(turn_started_at));
        if let Some(rec) = rec.as_ref() {
            let agent_label = state.session_instance(rec).display_slug();
            state.emit_tail(TailEvent::Turn {
                ts: now,
                project: rec.channel_h.clone(),
                agent: agent_label,
                session: rec.session_id.clone(),
                state: "idle".into(),
                elapsed_s,
            });
        }
        crate::tmux::ring_doorbells(state.clone());
    }
    Ok(serde_json::json!({ "ok": true }))
}
