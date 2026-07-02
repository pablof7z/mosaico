use super::*;
use crate::state::Session;

#[cfg(test)]
mod tests;

/// Build the wire `domain::Status` for a locally-hosted session from its row.
/// `title`/`activity`/`working` are the local pre-publish draft on the `sessions`
/// row; publishing turns them into a kind:30315 read back into `relay_status`.
pub(in crate::daemon::server) fn status_from_session(
    state: &Arc<DaemonState>,
    instance: &crate::identity::AgentInstance,
    rec: &Session,
    host: &str,
    now: u64,
) -> crate::domain::Status {
    let expires_at = now.saturating_add(state.status_ttl.as_secs());
    crate::domain::Status {
        agent: instance.agent_ref(),
        channels: status_channels(state, rec),
        session_id: crate::util::SessionId::new(rec.session_id.clone()),
        host: host.to_string(),
        title: rec.title.clone(),
        activity: rec.activity.clone(),
        busy: rec.working,
        rel_cwd: String::new(),
        expires_at: Some(expires_at),
    }
}

fn status_channels(state: &Arc<DaemonState>, rec: &Session) -> Vec<String> {
    let mut channels: Vec<String> = state
        .with_store(|s| {
            s.list_session_joined_channels(&rec.session_id)
                .unwrap_or_default()
        })
        .into_iter()
        .map(|(channel, _)| channel)
        .collect();
    if !rec.channel_h.is_empty() && !channels.iter().any(|c| c == &rec.channel_h) {
        channels.push(rec.channel_h.clone());
    }
    channels.sort();
    channels.dedup();
    channels
}

/// Reflect a just-published status into the local `relay_status` cache so liveness
/// is visible immediately (without waiting for the relay to echo the 30315 back).
fn cache_status(state: &Arc<DaemonState>, st: &crate::domain::Status, signer: &str, now: u64) {
    let fallback_expiration = now.saturating_add(state.status_ttl.as_secs());
    state.with_store(|s| {
        for channel in &st.channels {
            let row = crate::state::Status {
                pubkey: signer.to_string(),
                session_id: st.session_id.as_str().to_string(),
                channel_h: channel.clone(),
                slug: st.agent.slug.clone(),
                title: st.title.clone(),
                activity: st.activity.clone(),
                busy: st.busy,
                last_seen: now,
                updated_at: now,
                expiration: st.expires_at.unwrap_or(fallback_expiration),
            };
            s.upsert_status(&row).ok();
        }
    });
}

/// Heartbeat re-arm: every `HEARTBEAT_SECS`, re-publish the current kind:30315 for
/// every live locally-hosted session so its NIP-40 `expiration` is pushed forward
/// to `now + status_ttl`. Without this a live-but-idle session's relay event
/// would expire after the configured TTL and read as gone. This is the piece that
/// turns store-side freshness into relay liveness.
pub(in crate::daemon::server) fn spawn_status_heartbeat_publisher(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(crate::domain::HEARTBEAT_SECS));
        loop {
            tick.tick().await;
            let now = now_secs();
            let fresh_since = now.saturating_sub(state.status_ttl.as_secs());
            let sessions = state.with_store(|s| s.list_alive_sessions().unwrap_or_default());
            for rec in sessions {
                // Skip sessions that haven't heartbeat locally within the TTL (and
                // aren't mid-turn) — they're effectively gone.
                if !rec.working && rec.last_seen < fresh_since {
                    continue;
                }
                // Issue #98: derive signing key + wire identity from the session's
                // ONE authoritative agent-instance identity, so a heartbeat cannot
                // collapse two duplicate sessions back onto the durable author.
                let instance = state.session_instance(&rec);
                let base = match identity::load_or_create(
                    &config::edge_home(),
                    &instance.base_slug,
                    now,
                ) {
                    Ok(i) => i,
                    Err(_) => continue,
                };
                let keys = instance.signing_keys(&base.keys);
                let status = status_from_session(&state, &instance, &rec, &state.host, now);
                match state.provider.set_status(&status, &keys).await {
                    Ok(_eid) => {
                        let signer = keys.public_key().to_hex();
                        cache_status(&state, &status, &signer, now);
                    }
                    Err(e) => tracing::error!(
                        session = %rec.session_id,
                        error = %format!("{e:#}"),
                        "status_publish: set_status failed — presence not refreshed this tick"
                    ),
                }
            }
        }
    });
}

/// Drain the generic outbox: publish each queued signed event JSON via the
/// transport with a checked relay verdict, marking it published only after a
/// relay accepts it. Failed attempts stay pending with an error and retry count.
/// Woken instantly by `outbox_notify` on every enqueue, and polled every 2s as a
/// fallback.
pub(in crate::daemon::server) fn spawn_outbox_drainer(state: Arc<DaemonState>) {
    use nostr_sdk::prelude::{Event, JsonUtil};
    tokio::spawn(async move {
        loop {
            // Publish the backlog while we keep making progress (so a startup
            // burst clears fast); stop if a whole batch failed to avoid a tight spin.
            loop {
                let items = state.with_store(|s| s.peek_outbox(32).unwrap_or_default());
                if items.is_empty() {
                    break;
                }
                let mut progressed = false;
                for item in items {
                    match Event::from_json(&item.event_json) {
                        Ok(ev) => match state.transport.publish_event_checked(&ev).await {
                            Ok(_) => {
                                state.with_store(|s| s.mark_published(item.local_id).ok());
                                progressed = true;
                            }
                            Err(e) => {
                                state.with_store(|s| {
                                    s.mark_failed(item.local_id, &format!("{e:#}")).ok()
                                });
                            }
                        },
                        Err(e) => {
                            // A row we can never parse is dead; record the error.
                            // It stays pending but won't block other rows.
                            state.with_store(|s| {
                                s.mark_failed(item.local_id, &format!("bad event json: {e}"))
                                    .ok()
                            });
                        }
                    }
                }
                if !progressed {
                    break;
                }
            }
            tokio::select! {
                _ = state.outbox_notify.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            }
        }
    });
}

// ── session lifecycle ─────────────────────────────────────────────────────────
