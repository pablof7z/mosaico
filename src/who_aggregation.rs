//! Canonical store aggregation for the human and JSON `who` projections.

use crate::identity::SessionIdentity;
use crate::state::{Channel, ChannelMember, Profile, Session, SessionStanding, Status, Store};
use anyhow::{Context, Result};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

mod projection;

pub(crate) struct WhoAggregation {
    pub(crate) channels: Vec<Channel>,
    pub(crate) local_sessions: Vec<Session>,
    pub(crate) local_pubkeys: BTreeSet<String>,
    pub(crate) stopped_standing: Vec<SessionStanding>,
    pub(crate) backend_profiles: Vec<Profile>,
    pub(crate) local_spawnable: BTreeMap<String, (String, Option<String>)>,
    channels_by_id: BTreeMap<String, Channel>,
    members: BTreeMap<String, Vec<ChannelMember>>,
    statuses: BTreeMap<String, Vec<Status>>,
    profiles: BTreeMap<String, Profile>,
    identities: BTreeMap<String, SessionIdentity>,
    sessions_by_pubkey: BTreeMap<String, Session>,
    session_routes: BTreeMap<String, Vec<String>>,
    local_presence: BTreeMap<String, crate::session_presence::PublicPresence>,
    workspace_paths: BTreeMap<String, String>,
}

impl WhoAggregation {
    pub(crate) fn load(store: &Store, now: u64) -> Result<Self> {
        let channels = store
            .list_channels()
            .context("who aggregation: failed to list channels")?;
        let local_sessions = store
            .list_running_sessions()
            .context("who aggregation: failed to list live local sessions")?;
        let local_pubkeys = store
            .list_local_session_pubkeys()
            .context("who aggregation: failed to list local session pubkeys")?
            .into_iter()
            .collect();
        let backend_profiles = store
            .list_backend_profiles()
            .context("who aggregation: failed to list backend profiles")?;
        let local_spawnable = crate::session_host::spawnable_agents()
            .into_iter()
            .map(|(slug, command, byline)| (slug, (command, byline)))
            .collect();
        let stopped_standing = store
            .list_stopped_member_standing()
            .context("who aggregation: failed to list stopped session standing")?;
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
        referenced_pubkeys.extend(
            stopped_standing
                .iter()
                .map(|standing| standing.pubkey.clone()),
        );
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
        referenced_pubkeys.extend(
            backend_profiles
                .iter()
                .map(|profile| profile.pubkey.clone()),
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
        let local_presence = local_sessions
            .iter()
            .map(|session| {
                let published = statuses
                    .values()
                    .flatten()
                    .filter(|row| row.pubkey == session.pubkey)
                    .max_by_key(|row| row.updated_at);
                (
                    session.pubkey.clone(),
                    crate::session_presence::local(store, session, published),
                )
            })
            .collect();
        let session_routes = local_sessions
            .iter()
            .map(|session| {
                let routes = store
                    .list_session_routes(&session.pubkey)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(channel, _)| channel)
                    .collect();
                (session.pubkey.clone(), routes)
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
            stopped_standing,
            backend_profiles,
            local_spawnable,
            channels_by_id,
            members,
            statuses,
            profiles,
            identities,
            sessions_by_pubkey,
            session_routes,
            local_presence,
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

    pub(crate) fn local_session_presence(
        &self,
        session: &Session,
    ) -> crate::session_presence::PublicPresence {
        self.local_presence.get(&session.pubkey).cloned().unwrap_or(
            crate::session_presence::PublicPresence {
                state: crate::session_state::SessionState::Offline,
                state_since: session.stopped_at,
                title: session.title.clone(),
                activity: String::new(),
                observed_at: session.last_seen,
            },
        )
    }

    pub(crate) fn session_routes(&self, pubkey: &str) -> Vec<String> {
        self.session_routes.get(pubkey).cloned().unwrap_or_default()
    }

    pub(crate) fn backend_profiles_for_root(&self, root: Option<&str>) -> Vec<&Profile> {
        self.backend_profiles
            .iter()
            .filter(|profile| {
                root.is_none_or(|root| {
                    profile.workspaces.iter().any(|workspace| workspace == root)
                        && self
                            .members_for(root)
                            .iter()
                            .any(|member| member.pubkey == profile.pubkey && member.role == "admin")
                })
            })
            .collect()
    }
}
