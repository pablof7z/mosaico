//! `relay_channels` — kind:39000 group metadata cache.
//!
//! A channel and a channel are one abstraction; `parent` is the only distinction
//! (`""` = top-level root channel, set = session/task channel).

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
pub const CHANNEL_ABOUT_MAX_CHARS: usize = 80;
pub const ARCHIVED_CHANNEL_ABOUT_PREFIX: &str = "[ARCHIVED]";

pub fn is_archived_channel_about(about: &str) -> bool {
    about.starts_with(ARCHIVED_CHANNEL_ABOUT_PREFIX)
}

pub fn archived_channel_about(about: &str) -> String {
    let archived = if is_archived_channel_about(about) {
        about.to_string()
    } else if about.trim().is_empty() {
        ARCHIVED_CHANNEL_ABOUT_PREFIX.to_string()
    } else {
        format!("{ARCHIVED_CHANNEL_ABOUT_PREFIX} {about}")
    };
    archived.chars().take(CHANNEL_ABOUT_MAX_CHARS).collect()
}

impl Channel {
    pub fn is_archived(&self) -> bool {
        is_archived_channel_about(&self.about)
    }
}

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

    /// A local reservation for a channel name whose relay-authored kind:39000 has
    /// not materialized yet. This is not channel truth; it prevents two immediate
    /// session-start hooks for the same `(parent, name)` from minting two ids while
    /// daemon-side readiness is still in flight.
    pub fn channel_resolution_intent(&self, parent: &str, name: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT channel_h FROM channel_resolution_intents
                 WHERE parent=?1 AND name=?2",
                params![parent, name],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }

    pub fn reserve_channel_resolution_intent(
        &self,
        parent: &str,
        name: &str,
        proposed_channel_h: &str,
        now: u64,
    ) -> Result<String> {
        self.conn.execute(
            "INSERT OR IGNORE INTO channel_resolution_intents
                 (parent, name, channel_h, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![parent, name, proposed_channel_h, now],
        )?;
        self.channel_resolution_intent(parent, name)?
            .ok_or_else(|| anyhow::anyhow!("failed to reserve channel resolution intent"))
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

    pub fn is_archived_channel(&self, channel_h: &str) -> Result<bool> {
        Ok(self
            .get_channel(channel_h)?
            .map(|channel| channel.is_archived())
            .unwrap_or(false))
    }

    /// Root channels as read-model rows, ordered by stable channel id.
    pub fn list_root_channels(&self) -> Result<Vec<Channel>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM relay_channels WHERE parent='' ORDER BY channel_h"
        ))?;
        let rows = stmt.query_map([], row_to_channel)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Read-model metadata for a workspace/channel.
    pub fn channel_meta_read_model(&self, channel_h: &str) -> Result<Option<Channel>> {
        self.get_channel(channel_h)
    }

    /// The `parent` h-tag of a channel (`""` for a top-level root channel),
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

    /// Walk a known channel's `parent` links to its top-level root channel.
    /// Returns `None` when the channel or any ancestor is unknown; callers that
    /// use rootness for routing/admission must not silently treat that as root.
    pub fn root_channel_of(&self, channel_h: &str) -> Result<Option<String>> {
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

    /// True when the channel is a known top-level root channel (`parent`
    /// empty). Unknown channels are not root.
    pub fn is_root_channel(&self, channel_h: &str) -> Result<bool> {
        Ok(self
            .root_channel_of(channel_h)?
            .map(|root| root == channel_h)
            .unwrap_or(false))
    }

    /// True when the channel is known and belongs under a different top-level
    /// channel root. Unknown ancestry is not treated as a sub-channel.
    pub fn is_subchannel(&self, channel_h: &str) -> Result<bool> {
        Ok(self
            .root_channel_of(channel_h)?
            .map(|root| root != channel_h)
            .unwrap_or(false))
    }
}

#[cfg(test)]
mod tests;
