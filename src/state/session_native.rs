use super::*;

impl Store {
    /// Bind or rotate the one harness-native resume locator for a pubkey.
    pub fn set_native_resume_locator(
        &self,
        pubkey: &str,
        harness: &str,
        native_resume: &str,
        now: u64,
    ) -> Result<()> {
        if !self.session_exists(pubkey)? {
            anyhow::bail!("cannot bind native resume locator for unknown pubkey {pubkey}");
        }
        self.put_session_locator(harness, LOCATOR_NATIVE_RESUME, native_resume, pubkey, now)
    }
}
