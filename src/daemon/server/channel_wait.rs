use super::*;
use crate::state::{Message, Session};
use std::collections::HashSet;
use std::time::{Duration, Instant};

mod author;
use author::AuthorFilter;
mod registry;
pub(super) use registry::ActiveWaitRegistry;
#[cfg(test)]
#[path = "channel_wait/tests.rs"]
mod tests;

const MESSAGE_BATCH: u32 = 512;

#[derive(serde::Deserialize, Default)]
pub(super) struct WaitParams {
    timeout_secs: u64,
    #[serde(default)]
    channels: Vec<String>,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    reply_to: Option<String>,
    #[serde(default)]
    from_pubkeys: Vec<String>,
    #[serde(default)]
    from_labels: Vec<String>,
}

pub(super) async fn rpc_channel_wait(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let started = Instant::now();
    let p: WaitParams =
        serde_json::from_value(params.clone()).context("parsing channel_wait params")?;
    if p.timeout_secs == 0 {
        anyhow::bail!("wait duration must be at least 1 second");
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(p.timeout_secs);
    if p.from.is_some() && (!p.from_pubkeys.is_empty() || !p.from_labels.is_empty()) {
        anyhow::bail!("channel_wait author filters are mutually exclusive");
    }

    let rec = resolve_session_inner(
        state,
        &CallerAnchor::from_params(params),
        ResolveScope::Strict,
    )?;
    let (scopes, mut cursor, reply_to) = wait_scope_and_cursor(state, &rec, &p, deadline).await?;
    let channel_refs = state.with_store(|store| {
        scopes
            .iter()
            .map(|scope| crate::channel_ref::full_channel_ref(store, scope))
            .collect::<Vec<_>>()
    });
    let author_filter = AuthorFilter::from_params(state, &scopes, &p)?;
    let own_pubkeys = own_pubkeys(&rec);
    let backend_pubkey = state.backend_pubkey().unwrap_or_default();

    // The rowid cursor is captured before subscribing. A message inserted in
    // that tiny gap is recovered by the immediate post-subscribe drain.
    let mut rx = state.tail_subscribe();
    for scope in &scopes {
        if tokio::time::timeout_at(deadline, ensure_subscription(state, scope))
            .await
            .is_err()
        {
            return Ok(timeout_result(p.timeout_secs, &channel_refs));
        }
    }
    let _active_wait =
        registry::register(state, &rec, &scopes, &author_filter, reply_to.as_deref());

    let timeout = tokio::time::sleep_until(deadline);
    tokio::pin!(timeout);
    loop {
        if let Some(message) = drain_matching(
            state,
            &mut cursor,
            &scopes,
            reply_to.as_deref(),
            &author_filter,
            &own_pubkeys,
            &backend_pubkey,
        )? {
            return Ok(message_result(
                state,
                &message,
                &channel_refs,
                started.elapsed(),
            ));
        }

        tokio::select! {
            _ = &mut timeout => {
                return Ok(timeout_result(p.timeout_secs, &channel_refs));
            }
            event = rx.recv() => match event {
                Ok(TailEvent::Msg { .. }) => {}
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    anyhow::bail!("channel wait stream closed");
                }
            }
        }
    }
}

fn timeout_result(timeout_secs: u64, channels: &[String]) -> serde_json::Value {
    serde_json::json!({
        "outcome": "timeout",
        "timeout_secs": timeout_secs,
        "channels": channels,
    })
}

/// How long a reply wait will wait for its OWN message to come back before
/// giving up on it. The message was published through NMP, which injects the
/// accepted write into the subscription this daemon already holds (#1182), so
/// the round trip is local and sub-millisecond -- but it is asynchronous, and
/// `channel_send` returning is acceptance rather than materialization. Spending
/// a slice of the wait's own budget on it is honest; failing on a race is not.
const REPLY_ORIGIN_GRACE: Duration = Duration::from_secs(5);

async fn wait_scope_and_cursor(
    state: &Arc<DaemonState>,
    rec: &Session,
    params: &WaitParams,
    deadline: tokio::time::Instant,
) -> Result<(Vec<String>, i64, Option<String>)> {
    if let Some(reply_to) = params.reply_to.as_deref().filter(|id| !id.is_empty()) {
        if !params.channels.is_empty() || params.from.is_some() {
            anyhow::bail!("reply waits cannot also set channels or --from");
        }
        let original = await_own_message(state, reply_to, deadline)
            .await
            .with_context(|| format!("message not found for reply wait: {reply_to}"))?;
        let own = own_pubkeys(rec);
        if !own.contains(&original.author_pubkey) {
            anyhow::bail!("can only wait for replies to a message authored by this session");
        }
        let cursor = state
            .with_store(|store| store.message_rowid(&original.message_id))?
            .context("outbound message has no local arrival cursor")?;
        return Ok((
            vec![original.channel_h],
            cursor,
            original.native_event_id.or(Some(original.message_id)),
        ));
    }

    let scopes = resolve_joined_scopes(state, rec, &params.channels)?;
    let cursor = state.with_store(|store| store.latest_message_rowid())?;
    Ok((scopes, cursor, None))
}

/// Wait for the message a reply wait is anchored to, up to
/// [`REPLY_ORIGIN_GRACE`] or the wait's own deadline, whichever is sooner.
async fn await_own_message(
    state: &Arc<DaemonState>,
    message_id: &str,
    deadline: tokio::time::Instant,
) -> Option<Message> {
    let grace = (tokio::time::Instant::now() + REPLY_ORIGIN_GRACE).min(deadline);
    loop {
        if let Ok(Some(message)) = state.with_store(|store| store.get_message(message_id)) {
            return Some(message);
        }
        if tokio::time::Instant::now() >= grace {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn resolve_joined_scopes(
    state: &Arc<DaemonState>,
    rec: &Session,
    requested: &[String],
) -> Result<Vec<String>> {
    let joined = state.with_store(|store| store.list_session_routes(&rec.pubkey))?;
    let joined = joined
        .into_iter()
        .map(|(channel, _)| channel)
        .collect::<Vec<_>>();
    if joined.is_empty() {
        anyhow::bail!("this session has not joined any channels");
    }
    if requested.is_empty() {
        return Ok(joined);
    }
    let mut scopes = Vec::new();
    for reference in requested {
        let resolved = resolve_joined_reference(state, &joined, reference)?;
        if !scopes.contains(&resolved) {
            scopes.push(resolved);
        }
    }
    Ok(scopes)
}

fn resolve_joined_reference(
    state: &Arc<DaemonState>,
    joined: &[String],
    reference: &str,
) -> Result<String> {
    let reference = reference.trim();
    let joined_refs = state.with_store(|store| {
        joined
            .iter()
            .map(|channel| {
                (
                    channel.clone(),
                    crate::channel_ref::full_channel_ref(store, channel),
                )
            })
            .collect::<Vec<_>>()
    });
    // An exact full path against an already-joined channel.
    let matches = joined_refs
        .iter()
        .filter(|(_, full)| full.eq_ignore_ascii_case(reference))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [(channel, _)] => Ok(channel.clone()),
        [] => anyhow::bail!("this session has not joined channel {reference:?}"),
        many => anyhow::bail!(
            "channel reference {reference:?} is ambiguous among joined channels: {}",
            many.iter()
                .map(|(_, full)| full.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn own_pubkeys(rec: &Session) -> HashSet<String> {
    HashSet::from([rec.pubkey.clone()])
}

fn drain_matching(
    state: &Arc<DaemonState>,
    cursor: &mut i64,
    scopes: &[String],
    reply_to: Option<&str>,
    author_filter: &AuthorFilter,
    own_pubkeys: &HashSet<String>,
    backend_pubkey: &str,
) -> Result<Option<Message>> {
    loop {
        let rows = state.with_store(|store| store.messages_after_rowid(*cursor, MESSAGE_BATCH))?;
        if rows.is_empty() {
            return Ok(None);
        }
        let full_batch = rows.len() == MESSAGE_BATCH as usize;
        for (rowid, message) in rows {
            *cursor = rowid;
            if !scopes.contains(&message.channel_h) || own_pubkeys.contains(&message.author_pubkey)
            {
                continue;
            }
            let hidden = state.with_store(|store| {
                channel_read_tail::is_backend_row(store, backend_pubkey, &message)
            });
            if hidden || !author_filter.matches(state, &message) {
                continue;
            }
            if let Some(expected) = reply_to {
                let actual = state.with_store(|store| store.message_reply_target(&message))?;
                if actual.as_deref() != Some(expected) {
                    continue;
                }
            }
            return Ok(Some(message));
        }
        if !full_batch {
            return Ok(None);
        }
    }
}

fn message_result(
    state: &Arc<DaemonState>,
    message: &Message,
    channels: &[String],
    elapsed: Duration,
) -> serde_json::Value {
    let rendered = channel_read_tail::chat_row_to_json(state, message, false);
    serde_json::json!({
        "outcome": "message",
        "waited_secs": elapsed.as_secs(),
        "channels": channels,
        "message": rendered,
    })
}
