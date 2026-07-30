//! Read-only local-cache channel message search RPC.

use super::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

mod cursor;
use cursor::{decode_cursor, encode_cursor};

#[derive(Debug, Default, Deserialize)]
struct SearchParams {
    #[serde(default)]
    from: Vec<String>,
    #[serde(default)]
    to: Vec<String>,
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    channels: Vec<String>,
    since: Option<u64>,
    until: Option<u64>,
    limit: Option<u32>,
    cursor: Option<String>,
    /// Deliberately rejected. Channel `/` is the all-cache scope.
    workspace: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct SearchResponse {
    channels: Vec<SearchChannel>,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
struct SearchChannel {
    r#ref: String,
    messages: Vec<SearchMessage>,
}

#[derive(Debug, Serialize)]
struct SearchMessage {
    event_id: String,
    from: String,
    recipients: Vec<String>,
    body: String,
    created_at: u64,
}

pub(in crate::daemon::server) fn rpc_channel_search(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let params: SearchParams =
        serde_json::from_value(params.clone()).context("invalid channel_search parameters")?;
    validate_params(&params)?;
    let query = if let Some(cursor) = params.cursor.as_deref() {
        decode_cursor(cursor)?
    } else {
        query_from_params(state, &params)?
    };
    let page = state.with_store(|store| store.search_messages(&query))?;
    let next_cursor = page
        .next
        .as_ref()
        .map(|position| encode_cursor(&query, position))
        .transpose()?;
    let channels = render_groups(state, page.hits)?;
    Ok(serde_json::to_value(SearchResponse {
        channels,
        next_cursor,
    })?)
}

fn query_from_params(
    state: &Arc<DaemonState>,
    params: &SearchParams,
) -> Result<crate::state::MessageSearchQuery> {
    let limit = params
        .limit
        .unwrap_or(crate::state::MESSAGE_SEARCH_DEFAULT_LIMIT);
    anyhow::ensure!(
        (1..=crate::state::MESSAGE_SEARCH_MAX_LIMIT).contains(&limit),
        "limit must be between 1 and {}",
        crate::state::MESSAGE_SEARCH_MAX_LIMIT
    );

    let (from_pubkeys, to_pubkeys, channels) = state.with_store(|store| {
        Ok::<_, anyhow::Error>((
            resolve_identities(store, &params.from)?,
            resolve_identities(store, &params.to)?,
            resolve_channel_scopes(store, &params.channels)?,
        ))
    })?;
    let mut contains = params
        .contains
        .iter()
        .map(|value| value.to_lowercase())
        .collect::<Vec<_>>();
    contains.sort();
    contains.dedup();
    Ok(crate::state::MessageSearchQuery {
        channels,
        from_pubkeys,
        to_pubkeys,
        contains,
        since: params.since,
        until: params.until,
        limit,
        before: None,
        backend_pubkey: state.backend_pubkey(),
    })
}

fn validate_params(params: &SearchParams) -> Result<()> {
    anyhow::ensure!(
        params.workspace.is_none(),
        "--workspace is not supported; omit channels or use --channel /"
    );
    if params.cursor.is_some() {
        anyhow::ensure!(
            params.from.is_empty()
                && params.to.is_empty()
                && params.contains.is_empty()
                && params.channels.is_empty()
                && params.since.is_none()
                && params.until.is_none()
                && params.limit.is_none(),
            "--cursor resumes its bound query and cannot be combined with search filters"
        );
        return Ok(());
    }
    if let (Some(since), Some(until)) = (params.since, params.until) {
        anyhow::ensure!(since <= until, "--since must not be later than --until");
    }
    for (label, values) in [
        ("from", &params.from),
        ("to", &params.to),
        ("contains", &params.contains),
        ("channel", &params.channels),
    ] {
        anyhow::ensure!(
            values.iter().all(|value| !value.is_empty()),
            "{label} filter must not be empty"
        );
    }
    Ok(())
}

fn resolve_identities(store: &Store, selectors: &[String]) -> Result<Vec<String>> {
    let mut pubkeys = selectors
        .iter()
        .map(|selector| store.resolve_message_search_identity(selector))
        .collect::<Result<Vec<_>>>()?;
    pubkeys.sort();
    pubkeys.dedup();
    Ok(pubkeys)
}

fn resolve_channel_scopes(store: &Store, selectors: &[String]) -> Result<Vec<String>> {
    if selectors.is_empty() || selectors.iter().any(|selector| selector.trim() == "/") {
        return Ok(Vec::new());
    }
    let channels = store.list_channels()?;
    let mut by_parent: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for channel in &channels {
        by_parent
            .entry(channel.parent.as_str())
            .or_default()
            .push(channel.channel_h.as_str());
    }
    let mut selected = BTreeSet::new();
    for selector in selectors {
        absolute::require_full_path("--channel", selector)?;
        let root = match absolute::resolve_absolute_channel_ref(store, selector) {
            ChannelResolution::Unique(channel) => channel,
            ChannelResolution::NotFound => {
                anyhow::bail!("{}", absolute::describe_missing_channel(store, selector))
            }
        };
        let mut stack = vec![root];
        let mut visited = 0usize;
        while let Some(channel) = stack.pop() {
            visited += 1;
            anyhow::ensure!(
                visited <= 10_000,
                "channel subtree exceeds 10000 locally cached rows"
            );
            if !selected.insert(channel.clone()) {
                continue;
            }
            if let Some(children) = by_parent.get(channel.as_str()) {
                stack.extend(children.iter().map(|child| (*child).to_string()));
            }
        }
    }
    Ok(selected.into_iter().collect())
}

fn render_groups(
    state: &Arc<DaemonState>,
    hits: Vec<crate::state::MessageSearchHit>,
) -> Result<Vec<SearchChannel>> {
    state.with_store(|store| {
        let mut groups = Vec::<SearchChannel>::new();
        let mut indices = HashMap::<String, usize>::new();
        for hit in hits {
            let channel_h = hit.message.channel_h.clone();
            let index = match indices.get(&channel_h) {
                Some(index) => *index,
                None => {
                    let reference = crate::channel_ref::full_channel_ref(store, &channel_h);
                    anyhow::ensure!(
                        !reference.is_empty(),
                        "cached message {} has no complete public channel path",
                        hit.message.message_id
                    );
                    let index = groups.len();
                    groups.push(SearchChannel {
                        r#ref: reference,
                        messages: Vec::new(),
                    });
                    indices.insert(channel_h, index);
                    index
                }
            };
            let from = crate::fabric_context::refs::pubkey_ref(
                store,
                &hit.message.author_pubkey,
                &state.host,
            );
            let recipients = hit
                .recipients
                .iter()
                .map(|edge| {
                    crate::fabric_context::refs::pubkey_ref(
                        store,
                        &edge.recipient_pubkey,
                        &state.host,
                    )
                })
                .collect();
            groups[index].messages.push(SearchMessage {
                event_id: hit.message.message_id,
                from,
                recipients,
                body: crate::profile::rewrite_body_mentions(store, &hit.message.body),
                created_at: hit.message.created_at,
            });
        }
        Ok(groups)
    })
}

#[cfg(test)]
#[path = "channel_search/tests.rs"]
mod tests;
