use super::*;

#[path = "work_start_reaction.rs"]
pub(crate) mod work_start_reaction;

const CONTEXT_PROFILE_WARM_WINDOW_SECS: u64 = 4 * 60 * 60;

#[derive(serde::Deserialize, Default)]
pub(in crate::daemon::server) struct TurnStartParams {
    #[serde(default)]
    transcript: Option<String>,
}

pub(in crate::daemon::server) async fn rpc_turn_start(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: TurnStartParams =
        serde_json::from_value(params.clone()).context("parsing turn_start params")?;
    let anchor = CallerAnchor::from_params(params);
    if anchor.explicit.is_none()
        && anchor.pty_session.is_none()
        && anchor.harness_session.is_none()
        && anchor.watch_pid.is_none()
    {
        return Ok(serde_json::json!({
            "context": serde_json::Value::Null,
            "audit": {
                "kind": "turn_start",
                "skipped": "empty-session-id",
                "output": { "emitted": false, "bytes": 0, "text": null },
            },
        }));
    }
    // Hooks speak typed runtime locators while managed launches also carry their
    // assigned public session identity. Resolve either through the one canonical
    // caller-anchor path; a native id is never itself a session identity.
    // Read the previous
    // turn_started_at BEFORE opening the turn for audit/debug context; durable
    // snapshot-vs-delta gating lives on the session's seen_cursor.
    let before = resolve_session_inner(state, &anchor, ResolveScope::Strict).ok();
    let prev_started = before.as_ref().map(|r| r.turn_started_at).unwrap_or(0);

    let now = now_secs();
    let before = match before {
        Some(r) => r,
        None => {
            return Ok(serde_json::json!({
                "context": serde_json::Value::Null,
                "audit": {
                    "kind": "turn_start",
                    "skipped": "session-not-found",
                    "input_harness_session": anchor.harness_session,
                    "prev_turn_started_at": prev_started,
                    "output": { "emitted": false, "bytes": 0, "text": null },
                },
            }));
        }
    };
    let transcript_ref = p
        .transcript
        .as_deref()
        .filter(|x| !x.is_empty())
        .map(str::to_string);
    turn_lifecycle::drive_turn_started(state, &before, now, transcript_ref)
        .context("applying turn_start lifecycle projection")?;

    let rec = state
        .with_store(|s| s.get_session(&before.pubkey).ok().flatten())
        .unwrap_or(before);

    let instance = state.session_instance(&rec);
    let agent_label = instance.display_slug();

    // Emit Turn{working} for the live tail feed, keyed on the routing scope.
    state.emit_tail(TailEvent::Turn {
        ts: now,
        channel: rec.channel_h.clone(),
        agent: agent_label,
        session: rec.pubkey.clone(),
        state: "working".into(),
        elapsed_s: None,
    });

    schedule_context_profile_warm(state.clone(), rec.clone(), context_warm_since(&rec, now));

    // Assemble via the shared turn-context module so daemon and hook tests cannot
    // drift. The receipt is the graph's OWN dependency trace — it replaces the
    // hand-rolled turn_start_audit and is consistent with the render by construction.
    let backend_pubkey = state.backend_pubkey().unwrap_or_default();
    let mut turn = crate::turn_context::assemble_turn_start(
        &state.store,
        &rec,
        &backend_pubkey,
        &state.host,
        prev_started,
        &state.runtime.hook_contexts,
    )?;
    if let Some(nudge) = super::channel_move::maybe_nudge(state, &rec, now) {
        turn.append_advisory(&nudge, "channel-topology-nudge");
    }
    let audit = turn.receipt.to_json();
    record_hook_receipt(state, &turn);
    cursor::drive_cursor_request(state, &rec, turn.receipt.now.max(0) as u64, true)
        .context("applying cursor turn_start projection")?;
    work_start_reaction::publish_for_started_turn(state, &rec);
    let context = turn
        .text
        .map(serde_json::Value::String)
        .unwrap_or(serde_json::Value::Null);
    Ok(serde_json::json!({ "context": context, "audit": audit }))
}

/// Slice 8: persist the hook-context render's receipt (the "why this injected
/// shape" trace) keyed by `<session>:<kind>:<now>` so `explain hook:<session>@ts`
/// can replay it. Off the hot path — a failed insert is logged, never fatal.
fn record_hook_receipt(state: &Arc<DaemonState>, turn: &crate::turn_context::TurnContext) {
    let r = &turn.receipt;
    let created_at = crate::instrument::now_millis();
    let row = crate::state::receipts::NewReceipt {
        surface: "hook_context".into(),
        transaction_id: turn.transaction_id,
        revision: turn.revision,
        changed_summary: r.to_json().to_string(),
        commands: "[]".into(),
        artifact_ref: Some(format!("{}:{}:{}", r.pubkey, r.kind, r.now)),
        created_at,
    };
    state.with_store(|s| {
        crate::instrument::record_receipt(s, row);
    });
}

pub(in crate::daemon::server) async fn rpc_turn_check(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let rec = resolve_session(state, &CallerAnchor::from_params(params))?;
    let now = now_secs();
    let delta_since = cursor::drive_cursor_request(state, &rec, now, rec.is_working())
        .context("applying cursor turn_check projection")?;
    schedule_context_profile_warm(state.clone(), rec.clone(), delta_since.unwrap_or(now));
    let mut turn = crate::turn_context::assemble_turn_check(
        &state.store,
        &rec,
        &state.host,
        delta_since,
        now,
        &state.runtime.hook_contexts,
    )?;
    if let Some(nudge) = super::channel_move::maybe_nudge(state, &rec, now) {
        turn.append_advisory(&nudge, "channel-topology-nudge");
    }
    let audit = turn.receipt.to_json();
    record_hook_receipt(state, &turn);
    work_start_reaction::publish_for_started_turn(state, &rec);
    let context = turn
        .text
        .map(serde_json::Value::String)
        .unwrap_or(serde_json::Value::Null);
    Ok(serde_json::json!({ "context": context, "audit": audit }))
}

pub(in crate::daemon::server) async fn rpc_turn_end(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let anchor = CallerAnchor::from_params(params);
    if anchor.explicit.is_none()
        && anchor.pty_session.is_none()
        && anchor.harness_session.is_none()
        && anchor.watch_pid.is_none()
    {
        return Ok(serde_json::json!({ "ok": true }));
    }
    // Read working/turn_started_at BEFORE closing the turn so we can compute
    // elapsed. Runtime locators resolve to the canonical pubkey-owned row.
    let pre = resolve_session_inner(state, &anchor, ResolveScope::Strict).ok();
    let (was_working, turn_started_at) = pre
        .as_ref()
        .map(|r| (r.is_working(), r.turn_started_at))
        .unwrap_or((false, 0));
    let now = now_secs();
    if let Some(rec) = pre.as_ref() {
        turn_lifecycle::drive_turn_ended(state, rec, now)
            .context("applying turn_end lifecycle projection")?;
    }

    let rec = pre.as_ref().and_then(|session| {
        state
            .with_store(|s| s.get_session(&session.pubkey))
            .ok()
            .flatten()
    });

    if was_working {
        let elapsed_s = (turn_started_at > 0).then(|| now.saturating_sub(turn_started_at));
        if let Some(rec) = rec.as_ref() {
            let agent_label = state.session_instance(rec).display_slug();
            state.emit_tail(TailEvent::Turn {
                ts: now,
                channel: rec.channel_h.clone(),
                agent: agent_label,
                session: rec.pubkey.clone(),
                state: "idle".into(),
                elapsed_s,
            });
        }
        crate::session_host::ring_doorbells(state.clone());
    }
    // The turn is over. If it was a pty-injected mention that the agent never
    // answered via `channel send`, auto-publish its last transcript text as the
    // reply so the channel sees a response instead of silence.
    if let Some(rec) = rec.as_ref() {
        if let Some(pending) = auto_reply::take(&rec.pubkey) {
            let state = state.clone();
            let rec = rec.clone();
            tokio::spawn(async move {
                auto_reply::publish_last_response(&state, &rec, pending).await;
            });
        }
    }
    Ok(serde_json::json!({ "ok": true }))
}

fn context_warm_since(rec: &crate::state::Session, now: u64) -> u64 {
    if rec.seen_cursor == 0 {
        now.saturating_sub(CONTEXT_PROFILE_WARM_WINDOW_SECS)
    } else {
        rec.seen_cursor
    }
}

fn schedule_context_profile_warm(state: Arc<DaemonState>, rec: crate::state::Session, since: u64) {
    tokio::spawn(async move {
        warm_context_profiles(&state, &rec, since).await;
    });
}

async fn warm_context_profiles(state: &Arc<DaemonState>, rec: &crate::state::Session, since: u64) {
    let pubkeys = state.with_store(|s| context_profile_pubkeys(s, rec, since));
    crate::profile::warm(state, &pubkeys).await;
}

fn context_profile_pubkeys(
    store: &crate::state::Store,
    rec: &crate::state::Session,
    since: u64,
) -> Vec<String> {
    let mut pubkeys = Vec::new();
    for row in store
        .peek_pending_for_pubkey(&rec.pubkey)
        .unwrap_or_default()
    {
        pubkeys.push(row.from_pubkey);
        pubkeys.extend(crate::profile::body_mention_pubkeys(&row.body));
    }

    let channels = match store.list_session_routes(&rec.pubkey) {
        Ok(channels) => channels,
        Err(error) => {
            tracing::warn!(
                session = %rec.pubkey,
                %error,
                "profile warming skipped because session channel membership lookup failed"
            );
            Vec::new()
        }
    };
    for (channel_h, _) in channels {
        for member in store.list_channel_members(&channel_h).unwrap_or_default() {
            pubkeys.push(member.pubkey);
        }
        for ev in store
            .chat_for_channel(&channel_h, since, 10_000)
            .unwrap_or_default()
        {
            if ev.kind != crate::fabric::nip29::wire::KIND_CHAT as u32 || ev.pubkey == rec.pubkey {
                continue;
            }
            pubkeys.push(ev.pubkey);
            pubkeys.extend(crate::fabric_context::p_tag_pubkeys(&ev.tags_json));
            pubkeys.extend(crate::profile::body_mention_pubkeys(&ev.content));
        }
    }

    pubkeys.sort();
    pubkeys.dedup();
    pubkeys
}
