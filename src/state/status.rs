//! Session-status reads projected from the current NIP-29 view.
//!
//! The slug comes from the status event's own `slug` tag. When a session
//! omitted it, the view fills it from the kind:0 profile in the same level.

use super::*;

impl Store {
    pub fn get_status(&self, pubkey: &str, channel_h: &str) -> Result<Option<Status>> {
        Ok(self
            .view()
            .statuses()
            .iter()
            .find(|status| status.pubkey == pubkey && status.channel_h == channel_h)
            .cloned())
    }

    pub fn statuses_in_channel(&self, channel_h: &str) -> Result<Vec<Status>> {
        Ok(self
            .view()
            .statuses()
            .iter()
            .filter(|status| status.channel_h == channel_h)
            .cloned()
            .collect())
    }

    pub fn list_status_sessions(
        self: &Store,
        agent: Option<&str>,
        since: Option<u64>,
    ) -> Result<Vec<Status>> {
        Ok(self
            .view()
            .statuses()
            .iter()
            .filter(|status| agent.is_none_or(|agent| status.slug == agent))
            .filter(|status| since.is_none_or(|since| status.updated_at >= since))
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use crate::session_state::SessionState;
    use crate::state::{Profile, Status, Store, TestRelayDelivery};

    #[test]
    fn a_status_without_a_slug_takes_the_slug_of_its_profile() {
        let store = Store::open_memory().unwrap();
        store.install_test_relay_delivery(
            TestRelayDelivery::new()
                .profiles([Profile {
                    pubkey: "pk".into(),
                    name: "reviewer".into(),
                    slug: "reviewer".into(),
                    agent_slug: "reviewer".into(),
                    host: "laptop".into(),
                    is_backend: false,
                    agents: Vec::new(),
                    workspaces: Vec::new(),
                    updated_at: 1,
                }])
                .statuses([Status {
                    pubkey: "pk".into(),
                    channel_h: "chan".into(),
                    slug: String::new(),
                    title: String::new(),
                    activity: String::new(),
                    workspace: String::new(),
                    branch: String::new(),
                    state: SessionState::Idle,
                    state_since: 1,
                    last_seen: 1,
                    updated_at: 1,
                    expiration: 0,
                }]),
        );

        let status = store.get_status("pk", "chan").unwrap().unwrap();
        assert_eq!(status.slug, "reviewer");
        assert_eq!(
            store
                .list_status_sessions(Some("reviewer"), None)
                .unwrap()
                .len(),
            1
        );
    }
}
