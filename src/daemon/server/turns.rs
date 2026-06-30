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
        return Ok(serde_json::json!({ "context": serde_json::Value::Null }));
    }
    // Hooks speak the harness id; every mutator below resolves it to the canonical
    // session id internally, so passing the raw alias is correct. Read the previous
    // turn_started_at BEFORE opening the turn (first-turn detection).
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
        None => return Ok(serde_json::json!({ "context": serde_json::Value::Null })),
    };

    // Emit Turn{working} for the live tail feed, keyed on the routing scope.
    state.emit_tail(TailEvent::Turn {
        ts: now,
        project: rec.channel_h.clone(),
        agent: rec.agent_slug.clone(),
        session: rec.session_id.clone(),
        state: "working".into(),
        elapsed_s: None,
    });

    // Warm the kind:0 profile cache for identities the synchronous context renderer
    // needs to name: pending inbound senders + body mentions + channel members.
    let to_warm: Vec<String> = state.with_store(|s| {
        let mut v: Vec<String> = Vec::new();
        for r in s
            .drain_pending_for_session(&rec.session_id)
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

    // Assemble via the SHARED cli.rs function so the injected text is byte-identical
    // to the pre-daemon CLI and cannot drift.
    let backend_pubkey = state.backend_pubkey.clone().unwrap_or_default();
    let base = crate::cli::assemble_turn_start_context(
        &state.store,
        &rec,
        &backend_pubkey,
        &state.host,
        prev_started,
    );
    // Surface newly-available invitable agents (decision D) on a DELTA turn only
    // (never the first turn), keyed off the same per-session high-water mark so a
    // given new agent is announced once.
    let merged = if prev_started != 0 {
        merge_new_agents(base, prev_started, now)
    } else {
        base
    };
    let context = merged
        .map(serde_json::Value::String)
        .unwrap_or(serde_json::Value::Null);
    Ok(serde_json::json!({ "context": context }))
}

/// Append the "new agents available" delta section (decision D) to an assembled
/// context. Reads the LOCAL keystore (daemon-only — never on a unit-tested code
/// path) and surfaces agents created in `(since, now]`. Standalone → labelled
/// with the `[tenex-edge]` prefix; folded into an existing delta as a section.
fn merge_new_agents(base: Option<String>, since: u64, now: u64) -> Option<String> {
    let edge = crate::config::edge_home();
    let roster = crate::identity::list_invitable_agents(&edge);
    let section = crate::cli::new_agent_block(&roster, since, now);
    match (base, section) {
        (Some(b), Some(s)) => Some(format!("{b}\n\n{s}")),
        (Some(b), None) => Some(b),
        (None, Some(s)) => Some(format!("[tenex-edge] {s}")),
        (None, None) => None,
    }
}

#[derive(serde::Deserialize, Default)]
pub(in crate::daemon::server) struct TurnCheckParams {
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    env_session: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    group: Option<String>,
}

pub(in crate::daemon::server) fn rpc_turn_check(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: TurnCheckParams = serde_json::from_value(params.clone()).unwrap_or_default();
    let rec = resolve_session(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
        p.group.as_deref(),
    )?;
    let now = now_secs();
    // The sibling-session delta is rendered from the awareness high-water mark
    // (`seen_cursor`). We advance the cursor atomically — only the first of any
    // concurrent PostToolUse hooks wins the CAS; the rest get delta_since=None
    // and emit nothing, preventing duplicate injections from parallel tool calls.
    let delta_since = if rec.working {
        let old = rec.seen_cursor;
        let won = state
            .with_store(|s| s.try_advance_seen_cursor(&rec.session_id, old, now))
            .unwrap_or(false);
        won.then_some(old)
    } else {
        None
    };
    let base =
        crate::cli::assemble_turn_check_context(&state.store, &rec, &state.host, delta_since, now);
    // Same roster-on-change surfacing as turn_start, gated on the delta window.
    let merged = match delta_since {
        Some(since) => merge_new_agents(base, since, now),
        None => base,
    };
    let context = merged
        .map(serde_json::Value::String)
        .unwrap_or(serde_json::Value::Null);
    Ok(serde_json::json!({ "context": context }))
}

#[derive(serde::Deserialize)]
pub(in crate::daemon::server) struct TurnEndParams {
    session: String,
    /// The agent's turn output (last assistant text), read from the transcript by
    /// the stop hook. Published as kind:9 chat into the session's channel.
    #[serde(default)]
    reply: Option<String>,
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
    // elapsed and gate the reply publish (alias-resolving read).
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

    // Publish the agent's turn output as kind:9 chat into the channel where it
    // worked: per-session channels and explicit task channels publish final
    // replies; bare project sessions still do not spam the root group.
    //
    // Gated on `was_working` so it is IDEMPOTENT against duplicate stop hooks and
    // client retries: the transition above clears the turn, so a second turn_end
    // reads `was_working == false` and never republishes. Best-effort.
    if was_working {
        if let (Some(rec), Some(reply)) = (rec.as_ref(), p.reply.as_deref()) {
            let reply = reply.trim();
            // Publish into non-root (per-session / task) channels only.
            let publish_reply =
                state.with_store(|s| !s.is_root_channel(&rec.channel_h).unwrap_or(true));
            if !reply.is_empty() && publish_reply {
                let publish = publish_agent_reply(state, rec, reply);
                let debug = std::env::var("TENEX_EDGE_DEBUG").is_ok();
                match tokio::time::timeout(std::time::Duration::from_millis(1500), publish).await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) if debug => {
                        eprintln!("[daemon] agent reply publish skipped: {e:#}")
                    }
                    Err(_) if debug => eprintln!("[daemon] agent reply publish timed out"),
                    _ => {}
                }
            }
        }

        let now = now_secs();
        let elapsed_s = (turn_started_at > 0).then(|| now.saturating_sub(turn_started_at));
        if let Some(rec) = rec.as_ref() {
            state.emit_tail(TailEvent::Turn {
                ts: now,
                project: rec.channel_h.clone(),
                agent: rec.agent_slug.clone(),
                session: rec.session_id.clone(),
                state: "idle".into(),
                elapsed_s,
            });
        }
        crate::tmux::ring_doorbells(state.clone());
    }
    Ok(serde_json::json!({ "ok": true }))
}
