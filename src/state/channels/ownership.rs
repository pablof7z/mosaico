use super::{row_to_channel, Channel, Store, COLS};
use crate::state::Result;

impl Store {
    /// Whether this host can prove that `channel_h` belongs to its managed
    /// channel forest. Observing a relay group or holding its management key is
    /// deliberately insufficient: roots are proved by a local workspace
    /// binding and children by the durable name-resolution intent that minted
    /// their opaque id.
    pub fn is_managed_channel(&self, channel_h: &str) -> Result<bool> {
        let managed: i64 = self.conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM workspace_roots WHERE channel_h=?1
                 UNION ALL
                 SELECT 1 FROM channel_resolution_intents WHERE channel_h=?1
             )",
            [channel_h],
            |row| row.get(0),
        )?;
        Ok(managed != 0)
    }

    /// Materialized relay groups whose local root binding or child-resolution
    /// intent proves Mosaico ownership. Pending intents without relay metadata
    /// are excluded: configuration reconciliation must never fabricate a group
    /// merely because creation was once attempted.
    pub fn list_managed_channels(&self) -> Result<Vec<Channel>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {COLS} FROM relay_channels AS channel
             WHERE EXISTS (
                 SELECT 1 FROM workspace_roots AS root
                 WHERE root.channel_h=channel.channel_h
             ) OR EXISTS (
                 SELECT 1 FROM channel_resolution_intents AS intent
                 WHERE intent.channel_h=channel.channel_h
             )
             ORDER BY channel.channel_h"
        ))?;
        let rows = statement.query_map([], row_to_channel)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
