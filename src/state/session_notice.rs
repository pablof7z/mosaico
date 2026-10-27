//! Durable, generation-scoped progressive notice claims.

use super::*;

impl Store {
    /// Claim a notice code exactly once for one runtime generation.
    pub(crate) fn claim_session_notice(
        &self,
        pubkey: &str,
        runtime_generation: u64,
        code: &str,
        shown_at: u64,
    ) -> Result<bool> {
        if pubkey.is_empty() || runtime_generation == 0 || code.is_empty() {
            anyhow::bail!("session notice requires pubkey, generation, and code");
        }
        Ok(self.conn.execute(
            "INSERT OR IGNORE INTO session_notice
                 (pubkey, runtime_generation, code, shown_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![pubkey, runtime_generation, code, shown_at],
        )? == 1)
    }
}
