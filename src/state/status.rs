//! Session-status reads projected from the current NIP-29 view.
//!
//! The slug comes from the status event's own `slug` tag. A session that omitted
//! it is enriched from its kind:0 profile in the complete profile view: the
//! enrichment reads the stable exact-author demand alongside the status level.

use super::*;

impl Store {
    pub fn get_status(&self, pubkey: &str, channel_h: &str) -> Result<Option<Status>> {
        let mut status = self
            .statuses()
            .into_iter()
            .find(|status| status.pubkey == pubkey && status.channel_h == channel_h);
        if let Some(status) = status.as_mut() {
            self.enrich_status_slug(status);
        }
        Ok(status)
    }

    pub fn statuses_in_channel(&self, channel_h: &str) -> Result<Vec<Status>> {
        let mut statuses = self
            .statuses()
            .into_iter()
            .filter(|status| status.channel_h == channel_h)
            .collect::<Vec<_>>();
        for status in statuses.iter_mut() {
            self.enrich_status_slug(status);
        }
        Ok(statuses)
    }

    pub fn list_status_sessions(
        self: &Store,
        agent: Option<&str>,
        since: Option<u64>,
    ) -> Result<Vec<Status>> {
        let mut statuses = self
            .statuses()
            .into_iter()
            .filter(|status| agent.is_none_or(|agent| status.slug == agent))
            .filter(|status| since.is_none_or(|since| status.updated_at >= since))
            .collect::<Vec<_>>();
        for status in statuses.iter_mut() {
            self.enrich_status_slug(status);
        }
        Ok(statuses)
    }

    /// Fill a status's slug from its kind:0 profile when the status event omitted
    /// the `slug` tag. The complete profile view is the single kind:0 source.
    fn enrich_status_slug(&self, status: &mut Status) {
        if status.slug.is_empty() {
            if let Some(profile) = self
                .current_profiles()
                .into_iter()
                .find(|profile| profile.pubkey == status.pubkey)
            {
                status.slug = profile.slug;
            }
        }
    }
}
