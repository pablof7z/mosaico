//! Canonical store aggregation for both `who` renderers.
//!
//! CLI JSON/text and the agent XML tree project the same captured channel,
//! session, capability, and status facts. Renderer-specific layout stays out
//! of this module.

use crate::state::{
    AgentAvailability, Channel, ChannelMember, Session, Status, Store, StoreReader,
};
use anyhow::{Context, Result};
use std::cmp::Reverse;
use std::collections::BTreeMap;

pub(crate) struct WhoAggregation {
    pub(crate) channels: Vec<Channel>,
    pub(crate) local_sessions: Vec<Session>,
    pub(crate) agents: Vec<AgentAvailability>,
    now: u64,
    channels_by_id: BTreeMap<String, Channel>,
    members: BTreeMap<String, Vec<ChannelMember>>,
    statuses: BTreeMap<String, Vec<Status>>,
}

impl WhoAggregation {
    pub(crate) fn load(store: &Store, now: u64) -> Result<Self> {
        let channels = store
            .list_channels()
            .context("who aggregation: failed to list channels")?;
        let local_sessions = store
            .list_alive_sessions()
            .context("who aggregation: failed to list live local sessions")?;
        let agents = store
            .list_agent_roster()
            .context("who aggregation: failed to list agent capabilities")?;
        let channels_by_id = channels
            .iter()
            .cloned()
            .map(|channel| (channel.channel_h.clone(), channel))
            .collect();
        let mut members = BTreeMap::new();
        for channel in &channels {
            members.insert(
                channel.channel_h.clone(),
                store
                    .list_channel_members(&channel.channel_h)
                    .with_context(|| {
                        format!(
                            "who aggregation: failed to list members for {}",
                            channel.channel_h
                        )
                    })?,
            );
        }
        let mut statuses = BTreeMap::<String, Vec<Status>>::new();
        for status in store
            .list_status_sessions(None, None)
            .context("who aggregation: failed to read statuses")?
            .into_iter()
            .filter(|status| status.expiration >= now)
        {
            statuses
                .entry(status.channel_h.clone())
                .or_default()
                .push(status);
        }
        for rows in statuses.values_mut() {
            rows.sort_by_key(|status| Reverse(status.updated_at));
        }
        Ok(Self {
            channels,
            local_sessions,
            agents,
            now,
            channels_by_id,
            members,
            statuses,
        })
    }

    pub(crate) fn channel(&self, channel_h: &str) -> Option<&Channel> {
        self.channels_by_id.get(channel_h)
    }

    pub(crate) fn channel_name<'a>(&'a self, channel_h: &'a str) -> &'a str {
        self.channel(channel_h)
            .and_then(Channel::human_name)
            .unwrap_or(channel_h)
    }

    pub(crate) fn members_for(&self, channel_h: &str) -> &[ChannelMember] {
        self.members
            .get(channel_h)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(crate) fn statuses_for(&self, channel_h: &str) -> &[Status] {
        self.statuses
            .get(channel_h)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(crate) fn observed_state(&self, status: &Status) -> crate::session_state::SessionState {
        status.state.observed(status.expiration >= self.now)
    }

    pub(crate) fn status_text(&self, status: &Status) -> String {
        if self.observed_state(status).is_working() && !status.activity.trim().is_empty() {
            status.activity.trim().to_string()
        } else {
            status.title.trim().to_string()
        }
    }

    pub(crate) fn local_session_state(
        &self,
        store: StoreReader<'_>,
        session: &Session,
    ) -> crate::session_state::SessionState {
        let fresh = self.now.saturating_sub(session.last_seen) <= crate::session::STATUS_TTL_SECS;
        crate::session_state::SessionState::classify(
            fresh,
            session.working,
            store.has_live_delivery_path(session),
        )
    }

    pub(crate) fn agents_for_root(&self, root: Option<&str>) -> Vec<AgentAvailability> {
        self.agents
            .iter()
            .filter(|agent| root.is_none_or(|root| agent.channel_h == root))
            .cloned()
            .collect()
    }
}
