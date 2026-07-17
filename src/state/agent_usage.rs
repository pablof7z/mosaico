use super::*;

impl Store {
    /// Aggregate sessions whose latest activity falls inside the caller's
    /// rolling window. The canonical agent slug intentionally folds unresolved
    /// profile combinations back onto the profile they would launch.
    pub fn agent_usage_since(&self, since: u64) -> Result<Vec<AgentUsage>> {
        let mut stmt = self.conn.prepare(
            "SELECT agent_slug,
                    SUM(CASE WHEN MAX(created_at, last_seen) >= ?1 THEN 1 ELSE 0 END),
                    MAX(MAX(created_at, last_seen))
             FROM sessions
             WHERE agent_slug <> ''
             GROUP BY agent_slug
             ORDER BY agent_slug",
        )?;
        let rows = stmt.query_map([since], |row| {
            Ok(AgentUsage {
                agent_slug: row.get(0)?,
                recent_uses: row.get(1)?,
                last_used: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
