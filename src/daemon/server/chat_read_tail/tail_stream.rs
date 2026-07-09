use super::super::*;
use super::{chat_row_refs, stream_lag_error};
use crate::state::Message;

// ── tail (streaming) ──────────────────────────────────────────────────────────

/// Parameters for the `tail` RPC.
#[derive(serde::Deserialize, Default)]
pub(in crate::daemon::server) struct TailParams {
    #[serde(default)]
    channel: Option<String>,
    /// Number of backfill events (recent messages + roster snapshot), default 20.
    #[serde(default)]
    backfill: Option<u64>,
    /// Return only events after this unix timestamp.
    #[serde(default)]
    since: Option<u64>,
}

pub(in crate::daemon::server) async fn handle_tail<W: AsyncWriteExt + Unpin>(
    state: &Arc<DaemonState>,
    id: u64,
    params: &serde_json::Value,
    writer: &mut W,
) -> Result<()> {
    let p: TailParams = serde_json::from_value(params.clone()).unwrap_or_default();
    let channel = p.channel.clone();
    let backfill_n = p.backfill.unwrap_or(20);
    let since = p.since.unwrap_or(0);

    // Ensure the requested channel is in the union subscription.
    if let Some(pr) = &channel {
        let _ = ensure_subscription(state, pr).await;
    }

    // Subscribe BEFORE backfill so we don't miss events that arrive during query.
    let mut rx = state.tail_subscribe();

    {
        *state.open_clients.lock().unwrap() += 1;
    }
    let _guard = ClientGuard(state.clone());

    // ── Backfill ────────────────────────────────────────────────────────────
    if backfill_n > 0 {
        let backfill_events = build_backfill(state, channel.as_deref(), backfill_n, since);
        for ev in backfill_events {
            if write_json(writer, &Response::item(id, serde_json::to_value(&ev)?))
                .await
                .is_err()
            {
                let _ = write_json(writer, &Response::end(id)).await;
                return Ok(());
            }
        }
    }

    // ── Live stream ─────────────────────────────────────────────────────────
    loop {
        match rx.recv().await {
            Ok(ev) => {
                if tail_event_matches_channel(&ev, channel.as_deref())
                    && ev.ts() >= since
                    && write_json(writer, &Response::item(id, serde_json::to_value(&ev)?))
                        .await
                        .is_err()
                {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                let _ = write_json(writer, &stream_lag_error(id, "tail", skipped)).await;
                return Ok(());
            }
        }
    }
    let _ = write_json(writer, &Response::end(id)).await;
    Ok(())
}

/// True when the event belongs to the requested channel scope (or no filter).
pub(in crate::daemon::server) fn tail_event_matches_channel(
    ev: &TailEvent,
    channel: Option<&str>,
) -> bool {
    let Some(pr) = channel else {
        return true;
    };
    let ev_channel = match ev {
        TailEvent::Msg { channel, .. } => channel.as_str(),
        TailEvent::Sync { channel, .. } => channel.as_str(),
        TailEvent::Turn { channel, .. } => channel.as_str(),
        TailEvent::Status { channel, .. } => channel.as_str(),
        TailEvent::Join { channel, .. } => channel.as_str(),
        TailEvent::Leave { channel, .. } => channel.as_str(),
        TailEvent::Sess { channel, .. } => channel.as_str(),
        TailEvent::Proj { channel, .. } => channel.as_str(),
        // Profiles are cross-channel; always include.
        TailEvent::Profile { .. } => return true,
    };
    ev_channel == pr
}

/// Build the backfill event list from the materialized caches.
///
/// Returns recent chat lines from `messages` as `Msg` events + a roster
/// snapshot built from live `relay_status` rows (peers AND local agents read
/// identically) and this daemon's own live sessions, sorted ascending by time.
pub(in crate::daemon::server) fn build_backfill(
    state: &Arc<DaemonState>,
    channel: Option<&str>,
    limit: u64,
    since: u64,
) -> Vec<TailEvent> {
    let mut events: Vec<TailEvent> = Vec::new();
    let now = now_secs();
    let cap = limit.min(u32::MAX as u64) as u32;

    // ── Recent chat lines from messages ──────────────────────────────────────
    let chat_rows: Vec<Message> = state.with_store(|s| match channel {
        Some(pr) => s
            .chat_messages_for_channel(pr, since, cap)
            .unwrap_or_default(),
        None => s.recent_chat_messages(since, cap).unwrap_or_default(),
    });
    for row in chat_rows {
        let (from_slug, _, to_session) = chat_row_refs(state, &row);
        let to = state.with_store(|s| {
            s.message_recipients(&row.message_id)
                .unwrap_or_default()
                .into_iter()
                .next()
                .map(|r| pubkey_short(&r.recipient_pubkey))
                .unwrap_or_else(|| "channel-chat".to_string())
        });
        events.push(TailEvent::Msg {
            ts: row.created_at,
            channel: row.channel_h.clone(),
            from: from_slug,
            from_session: row.author_session.clone(),
            to,
            to_session,
            body: row.body.chars().take(200).collect(),
        });
    }

    // ── Roster snapshot: live status rows (peers + local agents) ─────────────
    if let Some(pr) = channel {
        let statuses = state.with_store(|s| s.live_status_for_channel(pr, now).unwrap_or_default());
        for st in statuses {
            let host = state
                .with_store(|s| s.get_profile(&st.pubkey))
                .ok()
                .flatten()
                .map(|p| p.host)
                .unwrap_or_default();
            events.push(TailEvent::Join {
                ts: st.last_seen,
                channel: st.channel_h.clone(),
                agent: st.slug.clone(),
                host,
                session: st.pubkey.clone(),
                rel_cwd: String::new(),
            });
            if !st.title.is_empty() || st.busy {
                events.push(TailEvent::Status {
                    ts: st.last_seen,
                    channel: st.channel_h.clone(),
                    agent: st.slug.clone(),
                    text: st.title.clone(),
                    active: st.busy,
                });
            }
        }
    }

    // ── This daemon's own live sessions as synthetic Sess/Turn events ────────
    let mine = state.with_store(|s| s.list_alive_sessions().unwrap_or_default());
    for rec in mine {
        if channel.map(|pr| rec.channel_h != pr).unwrap_or(false) {
            continue;
        }
        events.push(TailEvent::Sess {
            ts: rec.created_at,
            channel: rec.channel_h.clone(),
            agent: rec.agent_slug.clone(),
            session: rec.session_id.clone(),
            state: "start".into(),
            rel_cwd: String::new(),
        });
        if rec.working {
            events.push(TailEvent::Turn {
                ts: rec.turn_started_at,
                channel: rec.channel_h.clone(),
                agent: rec.agent_slug.clone(),
                session: rec.session_id.clone(),
                state: "working".into(),
                elapsed_s: None,
            });
        }
    }

    // Sort ascending by timestamp.
    events.sort_by_key(|e| e.ts());
    events
}
