//! Session-status reads projected directly from the current NMP row delivery.

use super::*;

impl Store {
    pub fn get_status(&self, pubkey: &str, channel_h: &str) -> Result<Option<Status>> {
        Ok(self.nmp_views.status(pubkey, channel_h))
    }

    pub fn statuses_in_channel(&self, channel_h: &str) -> Result<Vec<Status>> {
        Ok(self.nmp_views.statuses_in_channel(channel_h))
    }

    pub fn list_status_sessions(
        &self,
        agent: Option<&str>,
        since: Option<u64>,
    ) -> Result<Vec<Status>> {
        Ok(self.nmp_views.statuses(agent, since))
    }
}
