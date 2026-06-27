use super::*;

#[derive(serde::Deserialize)]
pub(in crate::daemon::server) struct SessionEndParams {
    session: String,
}

pub(in crate::daemon::server) fn rpc_session_end(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: SessionEndParams =
        serde_json::from_value(params.clone()).context("parsing session_end params")?;
    let rec = state.with_store(|s| s.get_session(&p.session).ok().flatten());
    let existed = rec.is_some();
    if let Some(ref rec) = rec {
        // Use the canonical id (rec.session_id), NOT the raw harness id p.session:
        // the runtime handle, the session_state row, and the registry are all keyed
        // by canonical — ending by alias would cancel/end nothing.
        cancel_session(state, &rec.session_id);

        // Release durable-slot reservation and any transient key before marking
        // the session dead. Fire-and-forget relay removal keeps session_end fast;
        // spawn_session cleanup will find the key gone and skip duplicate work.
        let session_key = state.release_session_signer(&rec.session_id);
        // Mark the (pubkey, h) route dead but KEEP the row: a later mention to
        // this ordinal resumes its bound native session (issue #47).
        state.with_store(|s| s.mark_identity_route_dead(&rec.session_id, now_secs()).ok());
        if let Some(sk) = session_key {
            let provider = state.provider.clone();
            let store = state.store.clone();
            // Remove from the session's CURRENT routing scope — its channel when
            // set (a `channels switch` moved it), else its per-session room — so
            // the NIP-29 member removal lands in the group the relay actually has
            // the agent in, not the room it minted at spawn.
            let scope = rec.route_scope().to_string();
            let session_pubkey = sk.public_key().to_hex();
            tokio::spawn(async move {
                let removed = provider.nip29_remove_member(&scope, &session_pubkey).await;
                // Mirror into the cache unconditionally: relay rejection of a
                // remove for a non-member is benign (idempotent), so always
                // clean up our local row to avoid stale membership.
                store
                    .lock()
                    .unwrap()
                    .remove_group_member(&scope, &session_pubkey)
                    .ok();
                if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                    eprintln!(
                        "[daemon] session-end NIP-29 remove {}: {}",
                        crate::util::pubkey_short(&session_pubkey),
                        if removed {
                            "accepted"
                        } else {
                            "skipped/failed (best-effort)"
                        },
                    );
                }
            });
        }

        state.with_store(|s| {
            // Finish the canonical aggregate (lifecycle=ended; title retained) so
            // the session surfaces as a 'gone' delta, AND mark the kept runtime row
            // dead. The final publish carries a fresh expiration and ages off.
            s.end_session(&rec.session_id, now_secs()).ok();
            s.mark_session_dead(&rec.session_id).ok();
            // Clear the DB routing row for this session's transient pubkey.
            s.remove_session_pubkeys_for_session(&rec.session_id).ok();
        });
        state.status_outbox_notify.notify_waiters();
        state.emit_tail(TailEvent::Sess {
            ts: now_secs(),
            project: rec.route_scope().to_string(),
            agent: rec.agent_slug.clone(),
            session: rec.session_id.clone(),
            state: "end".into(),
            rel_cwd: rec.rel_cwd.clone(),
        });
    }
    Ok(serde_json::json!({ "ended": existed }))
}
