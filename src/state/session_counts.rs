//! The narrow per-session facts agent counting reads, without full session rows.

use super::*;

pub(crate) struct SessionCountFacts {
    pub(crate) pubkey: String,
    pub(crate) named: bool,
    pub(crate) running: bool,
}

impl Store {
    /// The facts agent counting needs about every local session, without
    /// loading full session rows.
    pub(crate) fn list_session_count_facts(&self) -> Result<Vec<SessionCountFacts>> {
        let mut statement = self
            .conn
            .prepare("SELECT pubkey, agent_slug, runtime_state='running' FROM sessions")?;
        let rows = statement.query_map([], |row| {
            Ok(SessionCountFacts {
                pubkey: row.get(0)?,
                named: !row.get_ref(1)?.as_str()?.trim().is_empty(),
                running: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
