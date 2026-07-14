//! Remote session-handle reservations materialized from relay kind:0 profiles.

use anyhow::Result;
use rusqlite::{params, Connection};

pub(super) fn reserves_handle(
    conn: &Connection,
    handle: &str,
    except_pubkey: Option<&str>,
) -> Result<bool> {
    let except_pubkey = except_pubkey.unwrap_or("");
    Ok(conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM relay_profiles
             WHERE is_backend=0 AND agent_slug<>''
               AND (name=?1 OR slug=?1) AND pubkey<>?2
         )",
        params![handle, except_pubkey],
        |row| row.get(0),
    )?)
}
