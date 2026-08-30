//! Session-status reads projected from the current Fava row delivery.
//!
//! The slug comes from the status event's own `slug` tag. A session that omitted
//! it is enriched from its kind:0 profile via the retained profile feed (mosaico#837):
//! the enrichment moved here from `ObservedViews::state_status` so ObservedViews no longer
//! reads kind:0 from the mirror.

use super::*;

impl Store {
    pub fn get_status(&self, pubkey: &str, channel_h: &str) -> Result<Option<Status>> {
        let mut status = self.observed_views.status(pubkey, channel_h);
        if let Some(status) = status.as_mut() {
            self.enrich_status_slug(status);
        }
        Ok(status)
    }

    pub fn statuses_in_channel(&self, channel_h: &str) -> Result<Vec<Status>> {
        let mut statuses = self.observed_views.statuses_in_channel(channel_h);
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
        let mut statuses = self.observed_views.statuses(agent, since);
        for status in statuses.iter_mut() {
            self.enrich_status_slug(status);
        }
        Ok(statuses)
    }

    /// Fill a status's slug from its kind:0 profile when the status event omitted
    /// the `slug` tag. Replaces the `ObservedViews::state_status` profile fallback that
    /// read the mirror; the retained profile feed is the single kind:0 source.
    fn enrich_status_slug(&self, status: &mut Status) {
        if status.slug.is_empty() {
            if let Some(profile) = self.profile_feed.profile(&status.pubkey) {
                status.slug = profile.slug;
            }
        }
    }
}
