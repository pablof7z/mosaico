use super::{AgentAvailability, Channel, Profile, Session, Status, Store};
use anyhow::Result;

/// Read-only capability for callers that assemble views from the store.
#[derive(Clone, Copy)]
pub(crate) struct StoreReader<'a> {
    store: &'a Store,
}

impl Store {
    pub(crate) fn reader(&self) -> StoreReader<'_> {
        StoreReader { store: self }
    }
}

impl StoreReader<'_> {
    pub(crate) fn get_channel(self, channel_h: &str) -> Result<Option<Channel>> {
        self.store.get_channel(channel_h)
    }

    pub(crate) fn list_channels(self) -> Result<Vec<Channel>> {
        self.store.list_channels()
    }

    pub(crate) fn channel_parent(self, channel_h: &str) -> Result<Option<String>> {
        self.store.channel_parent(channel_h)
    }

    pub(crate) fn channel_project_root(self, channel_h: &str) -> Result<Option<String>> {
        self.store.channel_project_root(channel_h)
    }

    pub(crate) fn is_root_channel(self, channel_h: &str) -> Result<bool> {
        self.store.is_root_channel(channel_h)
    }

    pub(crate) fn get_profile(self, pubkey: &str) -> Result<Option<Profile>> {
        self.store.get_profile(pubkey)
    }

    pub(crate) fn resolve_slug_for_pubkey(self, pubkey: &str) -> Result<Option<String>> {
        self.store.resolve_slug_for_pubkey(pubkey)
    }

    pub(crate) fn list_identity_pubkeys(self) -> Result<Vec<String>> {
        self.store.list_identity_pubkeys()
    }

    pub(crate) fn list_agent_roster_for_channel(
        self,
        channel_h: &str,
    ) -> Result<Vec<AgentAvailability>> {
        self.store.list_agent_roster_for_channel(channel_h)
    }

    pub(crate) fn list_agent_roster(self) -> Result<Vec<AgentAvailability>> {
        self.store.list_agent_roster()
    }

    pub(crate) fn instance_identity_for_session(
        self,
        session_id: &str,
    ) -> Result<Option<crate::identity::AgentInstance>> {
        self.store.instance_identity_for_session(session_id)
    }

    pub(crate) fn list_alive_sessions(self) -> Result<Vec<Session>> {
        self.store.list_alive_sessions()
    }

    pub(crate) fn get_status(
        self,
        pubkey: &str,
        session_id: &str,
        channel_h: &str,
    ) -> Result<Option<Status>> {
        self.store.get_status(pubkey, session_id, channel_h)
    }

    pub(crate) fn live_status_for_channel(self, channel_h: &str, now: u64) -> Result<Vec<Status>> {
        self.store.live_status_for_channel(channel_h, now)
    }
}
