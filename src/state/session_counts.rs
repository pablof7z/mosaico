//! The narrow per-session fact agent counting reads, without full session rows.

use super::*;

impl Store {
    /// Local sessions that carry an agent slug, without loading full rows.
    pub(crate) fn list_named_session_pubkeys(&self) -> Result<Vec<String>> {
        let mut statement = self
            .conn
            .prepare("SELECT pubkey, agent_slug FROM sessions")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                !row.get_ref(1)?.as_str()?.trim().is_empty(),
            ))
        })?;
        let mut named = Vec::new();
        for row in rows {
            let (pubkey, is_named) = row?;
            if is_named {
                named.push(pubkey);
            }
        }
        Ok(named)
    }
}
