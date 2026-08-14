use super::*;

#[path = "work_start_reaction.rs"]
pub(crate) mod work_start_reaction;

fn hook_owns_lifecycle(admitted_transport: &str) -> bool {
    matches!(admitted_transport, "" | "pty")
}

pub(in crate::daemon::server) async fn rpc_turn_start(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
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
    let owns_lifecycle = hook_owns_lifecycle(&before.admitted_transport);
    if owns_lifecycle {
        turn_lifecycle::drive_turn_started(state, &before, now)
            .await
            .context("applying turn_start lifecycle projection")?;
    }

    let rec = state
        .with_store(|s| s.get_session(&before.pubkey).ok().flatten())
        .unwrap_or(before);

    let instance = state.session_instance(&rec);
    let agent_label = instance.display_slug();

    if owns_lifecycle {
        // Emit Turn{working} for the live tail feed, keyed on the routing scope.
        emit_turn_for_routes(state, &rec, now, &agent_label, "working", None);
    }

    // PTY inject is only confirmed when the harness user-prompt matches a
    // `submitted` row. Confirmed → `injected` (echo-suppress). Unconfirmed
    // submissions on a prompt-bearing turn roll back to `pending` so this
    // turn's hook path can deliver them. Without a prompt we leave
    // `submitted` alone (stale timeout / later UPS handles it).
    let prompt = params.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
    if owns_lifecycle && !prompt.is_empty() {
        if let Err(e) = state.with_store(|s| {
            s.confirm_submitted_from_prompt(&rec.pubkey, prompt, now)?;
            s.reenqueue_submitted(&rec.pubkey)?;
            Ok::<(), anyhow::Error>(())
        }) {
            tracing::error!(
                pubkey = %rec.pubkey,
                error = %e,
                "turn_start: PTY submission confirm/reenqueue failed"
            );
        }
    }

    // Assemble via the shared turn-context module so daemon and hook tests cannot
    // drift. The receipt is the graph's OWN dependency trace — it replaces the
    // hand-rolled turn_start_audit and is consistent with the render by construction.
    let backend_pubkey = state.backend_pubkey().unwrap_or_default();
    let mut turn = crate::turn_context::assemble_turn_start(
        &state.store,
        &rec,
        &backend_pubkey,
        &state.host(),
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
    if owns_lifecycle {
        work_start_reaction::publish_for_started_turn(state, &rec);
    }
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
    let mut turn = crate::turn_context::assemble_turn_check(
        &state.store,
        &rec,
        &state.host(),
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
    let (owns_lifecycle, was_working, turn_started_at) = pre
        .as_ref()
        .map(|r| {
            (
                hook_owns_lifecycle(&r.admitted_transport),
                r.is_working(),
                r.turn_started_at,
            )
        })
        .unwrap_or((false, false, 0));
    let now = now_secs();
    if owns_lifecycle {
        let rec = pre.as_ref().expect("lifecycle owner has resolved session");
        turn_lifecycle::drive_turn_ended(state, rec, now)
            .await
            .context("applying turn_end lifecycle projection")?;
    }

    let rec = pre.as_ref().and_then(|session| {
        state
            .with_store(|s| s.get_session(&session.pubkey))
            .ok()
            .flatten()
    });

    if owns_lifecycle && was_working {
        let elapsed_s = (turn_started_at > 0).then(|| now.saturating_sub(turn_started_at));
        if let Some(rec) = rec.as_ref() {
            let agent_label = state.session_instance(rec).display_slug();
            emit_turn_for_routes(state, rec, now, &agent_label, "idle", elapsed_s);
        }
        crate::session_host::ring_doorbells(state.clone());
    }
    Ok(serde_json::json!({ "ok": true }))
}

fn emit_turn_for_routes(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    at: u64,
    agent: &str,
    work_state: &str,
    elapsed_s: Option<u64>,
) {
    let routes = state
        .with_store(|store| store.list_session_routes(&rec.pubkey))
        .unwrap_or_default();
    for (channel, _) in routes {
        state.emit_tail(TailEvent::Turn {
            ts: at,
            channel,
            agent: agent.to_string(),
            session: rec.pubkey.clone(),
            state: work_state.to_string(),
            elapsed_s,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::hook_owns_lifecycle;

    #[test]
    fn hook_lifecycle_is_limited_to_unhosted_and_pty_sessions() {
        assert!(hook_owns_lifecycle(""));
        assert!(hook_owns_lifecycle("pty"));
        for managed in ["acp", "app-server", "pi-rpc"] {
            assert!(!hook_owns_lifecycle(managed), "{managed}");
        }
    }
}
