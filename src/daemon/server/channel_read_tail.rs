use super::chat_target::resolve_chat_target_provisioning;
use super::*;
use crate::state::Message;
use crate::util::{truncate_words, CHAT_RENDER_WORD_LIMIT};

mod backend_filter;
mod read_scope;
mod tail_stream;
#[cfg(test)]
mod tests;

pub(in crate::daemon::server) use backend_filter::is_backend_row;
use read_scope::{channel_read_scopes_for_store, ChatCursor};
pub(in crate::daemon::server) use tail_stream::handle_tail;

/// Upper bound on chat-log rows pulled per channel for a read (the slicing below
/// narrows to the requested window).
const CHANNEL_READ_CAP: u32 = 10_000;

#[derive(serde::Deserialize, Default)]
pub(in crate::daemon::server) struct ChatReadParams {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    since: Option<u64>,
    #[serde(default)]
    limit: Option<u64>,
    #[serde(default)]
    offset: Option<u64>,
    #[serde(default)]
    tail: bool,
    #[serde(default)]
    live: bool,
}

pub(in crate::daemon::server) async fn handle_channel_read<W: AsyncWriteExt + Unpin>(
    state: &Arc<DaemonState>,
    id: u64,
    params: &serde_json::Value,
    writer: &mut W,
) -> Result<()> {
    let p: ChatReadParams = serde_json::from_value(params.clone()).unwrap_or_default();
    if let Some(event_id) = p.id.as_deref().filter(|s| !s.trim().is_empty()) {
        let row = state
            .with_store(|s| s.get_message_by_prefix(event_id))
            .with_context(|| format!("reading chat message {event_id}"))?
            .with_context(|| format!("chat message not found: {event_id}"))?;
        let json = chat_row_to_json(state, &row, false);
        if write_json(writer, &Response::item(id, json)).await.is_ok() {
            let _ = write_json(writer, &Response::end(id)).await;
        }
        return Ok(());
    }

    let rec = resolve_session_inner(
        state,
        &CallerAnchor::from_params(params),
        ResolveScope::Channel,
    )?;
    let target =
        resolve_chat_target_provisioning(state, &rec, p.channel.as_deref(), "channel read").await?;
    let scope = target.channel_h;
    let since = p.since.unwrap_or(0);
    let offset = p.offset.unwrap_or(0);

    let _ = ensure_subscription(state, &scope).await;
    let read_scopes = channel_read_scopes(state, &scope);
    let mut rx = if p.live {
        Some(state.tail_subscribe())
    } else {
        None
    };
    let live_started_at = now_secs();
    let live_floor = live_started_at.max(since);

    let backend_pubkey = state.backend_pubkey().unwrap_or_default();
    let rows = state.with_store(|s| {
        let mut rows: Vec<Message> = read_scopes
            .iter()
            .flat_map(|sc| {
                s.chat_messages_for_channel(sc, since, CHANNEL_READ_CAP)
                    .unwrap_or_default()
            })
            .filter(|row| !is_backend_row(s, &backend_pubkey, row))
            .collect();
        rows.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.message_id.cmp(&b.message_id))
        });
        if p.tail {
            let limit = p.limit.unwrap_or(10) as usize;
            let start = rows
                .len()
                .saturating_sub(limit.saturating_add(offset as usize));
            let end = rows.len().saturating_sub(offset as usize);
            rows = rows.get(start..end).unwrap_or(&[]).to_vec();
        } else {
            let start = offset as usize;
            let end = p
                .limit
                .map(|limit| start.saturating_add(limit as usize))
                .unwrap_or(rows.len())
                .min(rows.len());
            rows = rows.get(start..end).unwrap_or(&[]).to_vec();
        }
        rows
    });
    let mut seen: std::collections::HashSet<String> =
        rows.iter().map(|r| r.message_id.clone()).collect();
    let mut cursors: std::collections::HashMap<String, ChatCursor> = read_scopes
        .iter()
        .map(|scope| (scope.clone(), ChatCursor::new(live_floor)))
        .collect();
    for row in &rows {
        cursors
            .entry(row.channel_h.clone())
            .or_insert_with(|| ChatCursor::new(live_floor))
            .observe(row);
    }

    for row in rows {
        let json = chat_row_to_json(state, &row, true);
        if write_json(writer, &Response::item(id, json)).await.is_err() {
            let _ = write_json(writer, &Response::end(id)).await;
            return Ok(());
        }
    }

    let Some(ref mut rx) = rx else {
        let _ = write_json(writer, &Response::end(id)).await;
        return Ok(());
    };

    loop {
        match rx.recv().await {
            Ok(TailEvent::Msg {
                channel: ev_channel,
                ..
            }) if read_scopes.contains(&ev_channel) => {
                let cursor = cursors
                    .entry(ev_channel.clone())
                    .or_insert_with(|| ChatCursor::new(live_floor))
                    .clone();
                let rows = state.with_store(|s| {
                    s.chat_messages_for_channel_after(
                        &ev_channel,
                        cursor.created_at,
                        &cursor.id,
                        CHANNEL_READ_CAP,
                    )
                    .unwrap_or_default()
                });
                for row in rows {
                    cursors
                        .entry(row.channel_h.clone())
                        .or_insert_with(|| ChatCursor::new(live_floor))
                        .observe(&row);
                    if !seen.insert(row.message_id.clone()) {
                        continue;
                    }
                    if state.with_store(|s| is_backend_row(s, &backend_pubkey, &row)) {
                        continue;
                    }
                    let json = chat_row_to_json(state, &row, true);
                    if write_json(writer, &Response::item(id, json)).await.is_err() {
                        let _ = write_json(writer, &Response::end(id)).await;
                        return Ok(());
                    }
                }
            }
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                let _ = write_json(
                    writer,
                    &stream_lag_error(id, "channel read --live", skipped),
                )
                .await;
                return Ok(());
            }
        }
    }
    let _ = write_json(writer, &Response::end(id)).await;
    Ok(())
}

fn channel_read_scopes(state: &Arc<DaemonState>, scope: &str) -> Vec<String> {
    state.with_store(|s| channel_read_scopes_for_store(s, scope))
}

/// Render a canonical chat message into the CLI's chat-line JSON, resolving the
/// author's slug from the materialized profile/session caches and rewriting any
/// `nostr:npub1…`/`nostr:nprofile1…` mentions in the body to `@name`, matching
/// the hook-injected fabric snapshot (`fabric_context::messages::message_row`).
pub(in crate::daemon::server) fn chat_row_to_json(
    state: &Arc<DaemonState>,
    row: &Message,
    truncate: bool,
) -> serde_json::Value {
    let (from_slug, host, mentioned_session) = chat_row_refs(state, row);
    let resolved_body = state.with_store(|s| crate::profile::rewrite_body_mentions(s, &row.body));
    let resolved_row = Message {
        body: resolved_body,
        ..row.clone()
    };
    chat_log_row_to_json(
        &resolved_row,
        &from_slug,
        &host,
        mentioned_session.as_deref(),
        truncate,
    )
}

pub(in crate::daemon::server) fn chat_log_row_to_json(
    row: &Message,
    from_slug: &str,
    host: &str,
    mentioned_session: Option<&str>,
    truncate: bool,
) -> serde_json::Value {
    let (body, truncated) = if truncate {
        truncate_words(&row.body, CHAT_RENDER_WORD_LIMIT)
    } else {
        (row.body.trim().to_string(), false)
    };
    serde_json::json!({
        "event_id": &row.message_id,
        "full_event_id": &row.message_id,
        "from_pubkey": &row.author_pubkey,
        "from_slug": from_slug,
        "host": host,
        "channel": &row.channel_h,
        "body": body,
        "truncated": truncated,
        "created_at": row.created_at,
        "from_session": row.author_session.as_deref().unwrap_or(""),
        "mentioned_session": mentioned_session.unwrap_or(""),
    })
}

fn chat_row_refs(state: &Arc<DaemonState>, row: &Message) -> (String, String, Option<String>) {
    let local_host = state.host.clone();
    state.with_store(|s| {
        let profile = s.get_profile(&row.author_pubkey).ok().flatten();
        let session = row
            .author_session
            .as_deref()
            .and_then(|sid| s.get_session(sid).ok().flatten());
        let from_slug = profile
            .as_ref()
            .map(|p| p.slug.as_str())
            .filter(|slug| !slug.is_empty())
            .or_else(|| session.as_ref().map(|rec| rec.agent_slug.as_str()))
            .filter(|slug| !slug.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| pubkey_short(&row.author_pubkey));
        // A whitelisted human operator has no session/host of their own — render
        // them bare (empty host) rather than falling through to the `?`
        // placeholder, mirroring the bare `<@name>` used for terminal mentions
        // in `injection::render_terminal_mention`.
        let is_whitelisted = state
            .whitelisted_pubkeys()
            .iter()
            .any(|w| w.eq_ignore_ascii_case(&row.author_pubkey));
        let host = if is_whitelisted {
            String::new()
        } else {
            profile
                .as_ref()
                .map(|p| p.host.clone())
                .filter(|h| !h.is_empty())
                .or_else(|| session.as_ref().map(|_| local_host))
                .unwrap_or_default()
        };
        let mentioned_session = s
            .message_recipients(&row.message_id)
            .unwrap_or_default()
            .into_iter()
            .find_map(|r| r.target_session);
        (from_slug, host, mentioned_session)
    })
}

fn stream_lag_error(id: u64, stream: &str, skipped: u64) -> Response {
    Response::err(
        id,
        "stream_lagged",
        format!(
            "{stream} dropped {skipped} live event(s); reconnect to resume from persisted history"
        ),
    )
}
