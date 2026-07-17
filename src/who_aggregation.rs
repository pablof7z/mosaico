//! Canonical store aggregation for both `who` renderers.
//!
//! CLI JSON/text and the agent XML tree project the same captured channel,
//! session, capability, and status facts. Renderer-specific layout stays out
//! of this module.

use crate::identity::SessionIdentity;
use crate::state::session_claims::SessionClaim;
use crate::state::{AgentAvailability, Channel, ChannelMember, Profile, Session, Status, Store};
use anyhow::{Context, Result};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

mod projection;

pub(crate) struct WhoAggregation {
    pub(crate) channels: Vec<Channel>,
    pub(crate) local_sessions: Vec<Session>,
    pub(crate) local_pubkeys: BTreeSet<String>,
    pub(crate) claims: Vec<SessionClaim>,
    pub(crate) agents: Vec<AgentAvailability>,
    now: u64,
    channels_by_id: BTreeMap<String, Channel>,
    members: BTreeMap<String, Vec<ChannelMember>>,
    statuses: BTreeMap<String, Vec<Status>>,
    profiles: BTreeMap<String, Profile>,
    identities: BTreeMap<String, SessionIdentity>,
    sessions_by_pubkey: BTreeMap<String, Session>,
    local_states: BTreeMap<String, crate::session_state::SessionState>,
    workspace_paths: BTreeMap<String, String>,
}

impl WhoAggregation {
    pub(crate) fn load(store: &Store, now: u64) -> Result<Self> {
        let channels = store
            .list_channels()
            .context("who aggregation: failed to list channels")?;
        let local_sessions = store
            .list_alive_sessions()
            .context("who aggregation: failed to list live local sessions")?;
        let local_pubkeys = store
            .list_local_session_pubkeys()
            .context("who aggregation: failed to list local session pubkeys")?
            .into_iter()
            .collect();
        let agents = store
            .list_agent_roster()
            .context("who aggregation: failed to list agent capabilities")?;
        let claims = store
            .list_active_session_claims(now)
            .context("who aggregation: failed to list active session claims")?;
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
            .filter(|status| status.expiration == 0 || status.expiration >= now)
        {
            statuses
                .entry(status.channel_h.clone())
                .or_default()
                .push(status);
        }
        for rows in statuses.values_mut() {
            rows.sort_by_key(|status| Reverse(status.updated_at));
        }
        let mut referenced_pubkeys = BTreeSet::new();
        referenced_pubkeys.extend(local_sessions.iter().map(|session| session.pubkey.clone()));
        referenced_pubkeys.extend(claims.iter().map(|claim| claim.pubkey.clone()));
        referenced_pubkeys.extend(
            members
                .values()
                .flatten()
                .map(|member| member.pubkey.clone()),
        );
        referenced_pubkeys.extend(
            statuses
                .values()
                .flatten()
                .map(|status| status.pubkey.clone()),
        );
        let mut profiles = BTreeMap::new();
        let mut identities = BTreeMap::new();
        let mut sessions_by_pubkey = BTreeMap::new();
        for pubkey in referenced_pubkeys {
            if let Some(profile) = store
                .get_profile(&pubkey)
                .with_context(|| format!("who aggregation: failed to read profile {pubkey}"))?
            {
                profiles.insert(pubkey.clone(), profile);
            }
            if let Some(identity) = store.session_identity(&pubkey).with_context(|| {
                format!("who aggregation: failed to read session identity {pubkey}")
            })? {
                identities.insert(pubkey.clone(), identity);
            }
            if let Some(session) = store
                .get_session(&pubkey)
                .with_context(|| format!("who aggregation: failed to read session {pubkey}"))?
            {
                sessions_by_pubkey.insert(pubkey, session);
            }
        }
        let local_states = local_sessions
            .iter()
            .map(|session| {
                let fresh =
                    now.saturating_sub(session.last_seen) <= crate::session::STATUS_TTL_SECS;
                let state = crate::session_state::SessionState::classify(
                    fresh,
                    session.working,
                    crate::session_host::session_has_live_delivery_path(store, session),
                );
                (session.pubkey.clone(), state)
            })
            .collect();
        let workspace_paths = store
            .list_workspace_bindings()
            .context("who aggregation: failed to list workspace bindings")?
            .into_iter()
            .map(|binding| (binding.channel_h, binding.abs_path))
            .collect();
        Ok(Self {
            channels,
            local_sessions,
            local_pubkeys,
            claims,
            agents,
            now,
            channels_by_id,
            members,
            statuses,
            profiles,
            identities,
            sessions_by_pubkey,
            local_states,
            workspace_paths,
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
        session: &Session,
    ) -> crate::session_state::SessionState {
        self.local_states
            .get(&session.pubkey)
            .copied()
            .unwrap_or(crate::session_state::SessionState::Offline)
    }

    pub(crate) fn agents_for_root(&self, root: Option<&str>) -> Vec<AgentAvailability> {
        self.agents
            .iter()
            .filter(|agent| root.is_none_or(|root| agent.channel_h == root))
            .cloned()
            .collect()
    }
}
