use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SearchCursor {
    version: u8,
    channels: Vec<String>,
    from_pubkeys: Vec<String>,
    to_pubkeys: Vec<String>,
    contains: Vec<String>,
    since: Option<u64>,
    until: Option<u64>,
    limit: u32,
    backend_pubkey: Option<String>,
    created_at: u64,
    message_id: String,
}

pub(super) fn encode_cursor(
    query: &crate::state::MessageSearchQuery,
    position: &crate::state::MessageSearchPosition,
) -> Result<String> {
    let cursor = SearchCursor {
        version: 1,
        channels: query.channels.clone(),
        from_pubkeys: query.from_pubkeys.clone(),
        to_pubkeys: query.to_pubkeys.clone(),
        contains: query.contains.clone(),
        since: query.since,
        until: query.until,
        limit: query.limit,
        backend_pubkey: query.backend_pubkey.clone(),
        created_at: position.created_at,
        message_id: position.message_id.clone(),
    };
    Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&cursor)?))
}

pub(super) fn decode_cursor(encoded: &str) -> Result<crate::state::MessageSearchQuery> {
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .context("invalid search cursor encoding")?;
    let cursor: SearchCursor =
        serde_json::from_slice(&bytes).context("invalid search cursor payload")?;
    anyhow::ensure!(cursor.version == 1, "unsupported search cursor version");
    anyhow::ensure!(
        !cursor.message_id.is_empty(),
        "search cursor has an empty message id"
    );
    anyhow::ensure!(
        (1..=crate::state::MESSAGE_SEARCH_MAX_LIMIT).contains(&cursor.limit),
        "search cursor has an invalid limit"
    );
    Ok(crate::state::MessageSearchQuery {
        channels: cursor.channels,
        from_pubkeys: cursor.from_pubkeys,
        to_pubkeys: cursor.to_pubkeys,
        contains: cursor.contains,
        since: cursor.since,
        until: cursor.until,
        limit: cursor.limit,
        before: Some(crate::state::MessageSearchPosition {
            created_at: cursor.created_at,
            message_id: cursor.message_id,
        }),
        backend_pubkey: cursor.backend_pubkey,
    })
}
