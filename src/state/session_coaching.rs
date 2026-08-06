//! Durable, generation-scoped progressive coaching claims.

use super::*;

impl Store {
    /// Claim a coaching code exactly once for one runtime generation.
    pub(crate) fn claim_session_coaching(
        &self,
        pubkey: &str,
        runtime_generation: u64,
        code: &str,
        shown_at: u64,
    ) -> Result<bool> {
        if pubkey.is_empty() || runtime_generation == 0 || code.is_empty() {
            anyhow::bail!("session coaching requires pubkey, generation, and code");
        }
        Ok(self.conn.execute(
            "INSERT OR IGNORE INTO session_coaching
                 (pubkey, runtime_generation, code, shown_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![pubkey, runtime_generation, code, shown_at],
        )? == 1)
    }
}
