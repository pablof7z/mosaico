use super::{Channel, Store};
use crate::state::Result;
use std::collections::BTreeSet;

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

    /// NMP-observed groups whose local root binding or child-resolution intent
    /// proves Mosaico ownership. Pending intents without NMP metadata are
    /// excluded: configuration reconciliation must never fabricate a group
    /// merely because creation was once attempted.
    pub fn list_managed_channels(&self) -> Result<Vec<Channel>> {
        let mut statement = self.conn.prepare(
            "SELECT channel_h FROM workspace_roots
             UNION
             SELECT channel_h FROM channel_resolution_intents",
        )?;
        let managed = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<BTreeSet<_>>>()?;
        Ok(self.nmp_views.with_groups(|groups| {
            groups
                .list_channels()
                .into_iter()
                .filter(|channel| managed.contains(&channel.channel_h))
                .collect()
        }))
    }
}
