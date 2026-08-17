//! Profile reads projected directly from the current NMP row delivery.

use super::*;

impl Store {
    pub fn get_profile(&self, pubkey: &str) -> Result<Option<Profile>> {
        Ok(self.profile_feed.profile(pubkey))
    }

    pub fn list_backend_profiles(&self) -> Result<Vec<Profile>> {
        Ok(self.nmp_views.backend_profiles())
    }

    pub fn resolve_agent_pubkey(&self, slug: &str, host: &str) -> Result<Option<String>> {
        Ok(self.nmp_views.resolve_agent_pubkey(slug, host))
    }

    pub fn resolve_profile_handle_pubkey(&self, handle: &str) -> Result<Option<String>> {
        self.nmp_views.resolve_profile_handle_pubkey(handle)
    }

    pub fn pubkey_for_backend_label(&self, backend_label: &str) -> Result<Option<String>> {
        Ok(self.nmp_views.pubkey_for_backend_label(backend_label))
    }

    pub fn resolve_slug_for_pubkey(&self, pubkey: &str) -> Result<Option<String>> {
        Ok(self.nmp_views.slug_for_pubkey(pubkey))
    }
}
