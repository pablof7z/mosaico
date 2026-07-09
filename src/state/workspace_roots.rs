//! `workspace_roots` — local filesystem map (channel id -> abs path on THIS
//! machine). The one fact about a channel's workspace that is genuinely
//! machine-local.

use super::*;

/// Local filesystem binding for a channel's workspace on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceBinding {
    pub channel_h: String,
    pub abs_path: String,
    pub updated_at: u64,
}

fn row_to_workspace(row: &rusqlite::Row) -> rusqlite::Result<WorkspaceBinding> {
    Ok(WorkspaceBinding {
        channel_h: row.get(0)?,
        abs_path: row.get(1)?,
        updated_at: row.get(2)?,
    })
}

impl Store {
    /// Record (or update) the absolute on-disk workspace path for a channel.
    pub fn upsert_workspace(&self, channel_h: &str, abs_path: &str, updated_at: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO workspace_roots (channel_h, abs_path, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(channel_h) DO UPDATE SET
                 abs_path=excluded.abs_path, updated_at=excluded.updated_at",
            params![channel_h, abs_path, updated_at],
        )?;
        Ok(())
    }

    /// The absolute workspace path for a channel on this machine, if known.
    pub fn workspace_path(&self, channel_h: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT abs_path FROM workspace_roots WHERE channel_h=?1",
                params![channel_h],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }

    /// The local filesystem workspace binding row for a channel on this machine.
    pub fn workspace_binding(&self, channel_h: &str) -> Result<Option<WorkspaceBinding>> {
        Ok(self
            .conn
            .query_row(
                "SELECT channel_h, abs_path, updated_at FROM workspace_roots WHERE channel_h=?1",
                params![channel_h],
                row_to_workspace,
            )
            .optional()?)
    }

    /// All locally known channel ids with on-disk workspace bindings.
    pub fn list_workspace_bindings(&self) -> Result<Vec<WorkspaceBinding>> {
        let mut stmt = self.conn.prepare(
            "SELECT channel_h, abs_path, updated_at FROM workspace_roots
             ORDER BY channel_h",
        )?;
        let rows = stmt
            .query_map([], row_to_workspace)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}
