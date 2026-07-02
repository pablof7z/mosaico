//! `relay_channels` — kind:39000 group metadata cache.
//!
//! A channel and a project are one abstraction; `parent` is the only distinction
//! (`""` = top-level project channel, set = session/task channel).

use super::*;

fn row_to_channel(row: &rusqlite::Row) -> rusqlite::Result<Channel> {
    Ok(Channel {
        channel_h: row.get(0)?,
        name: row.get(1)?,
        about: row.get(2)?,
        parent: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

const COLS: &str = "channel_h, name, about, parent, created_at, updated_at";
const MAX_CHANNEL_PARENT_DEPTH: usize = 16;

impl Store {
    /// Materialize a kind:39000 metadata event. Newer `created_at` wins; an older
    /// event for the same channel is ignored (NIP-01 replacement semantics).
    pub fn upsert_channel(
        &self,
        channel_h: &str,
        name: &str,
        about: &str,
        parent: &str,
        created_at: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO relay_channels (channel_h, name, about, parent, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(channel_h) DO UPDATE SET
                 name=excluded.name, about=excluded.about, parent=excluded.parent,
                 created_at=excluded.created_at, updated_at=excluded.updated_at
             WHERE excluded.created_at >= relay_channels.created_at",
            params![channel_h, name, about, parent, created_at],
        )?;
        Ok(())
    }

    /// The opaque `channel_h` for a `name` within `parent`. The identity of a
    /// channel is the `(parent, name)` pair; the `channel_h` is the durable key.
    /// When pre-existing duplicate rows share that pair, the most-recently-updated
    /// wins (the tiebreaker only matters for legacy dupes — new creates dedupe on
    /// `(parent, name)`). `None` when no channel by that name exists under `parent`.
    pub fn channel_id_for_name(&self, parent: &str, name: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT channel_h FROM relay_channels WHERE parent=?1 AND name=?2 \
                 ORDER BY updated_at DESC LIMIT 1",
                params![parent, name],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }

    /// Fetch one channel's metadata.
    pub fn get_channel(&self, channel_h: &str) -> Result<Option<Channel>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLS} FROM relay_channels WHERE channel_h=?1"),
                params![channel_h],
                row_to_channel,
            )
            .optional()?)
    }

    /// All known channels, newest metadata first.
    pub fn list_channels(&self) -> Result<Vec<Channel>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM relay_channels ORDER BY updated_at DESC"
        ))?;
        let rows = stmt.query_map([], row_to_channel)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The `parent` h-tag of a channel (`""` for a top-level project channel),
    /// or `None` if the channel is unknown.
    pub fn channel_parent(&self, channel_h: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT parent FROM relay_channels WHERE channel_h=?1",
                params![channel_h],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }

    /// Walk a known channel's `parent` links to its top-level project channel.
    /// Returns `None` when the channel or any ancestor is unknown; callers that
    /// use rootness for routing/admission must not silently treat that as root.
    pub fn channel_project_root(&self, channel_h: &str) -> Result<Option<String>> {
        if self.get_channel(channel_h)?.is_none() {
            return Ok(None);
        }

        let mut cur = channel_h.to_string();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..MAX_CHANNEL_PARENT_DEPTH {
            if !seen.insert(cur.clone()) {
                anyhow::bail!("channel parent cycle detected at {cur}");
            }
            let Some(parent) = self.channel_parent(&cur)? else {
                return Ok(None);
            };
            if parent.is_empty() {
                return Ok(Some(cur));
            }
            if self.get_channel(&parent)?.is_none() {
                return Ok(None);
            }
            cur = parent;
        }
        anyhow::bail!(
            "channel parent chain exceeds {MAX_CHANNEL_PARENT_DEPTH} links at {channel_h}"
        );
    }

    /// True when the channel is a known top-level project channel (`parent`
    /// empty). Unknown channels are not root.
    pub fn is_root_channel(&self, channel_h: &str) -> Result<bool> {
        Ok(self
            .channel_project_root(channel_h)?
            .map(|root| root == channel_h)
            .unwrap_or(false))
    }

    /// True when the channel is known and belongs under a different top-level
    /// project root. Unknown ancestry is not treated as a sub-channel.
    pub fn is_subchannel(&self, channel_h: &str) -> Result<bool> {
        Ok(self
            .channel_project_root(channel_h)?
            .map(|root| root != channel_h)
            .unwrap_or(false))
    }
}
