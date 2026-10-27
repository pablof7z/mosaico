use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::state::{ChannelMember, Store};

/// Count-relevant identity facts after all store-specific evidence has been
/// normalized. Both channel-list/MCP and fabric-context projection consume
/// this exact value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MemberFacts {
    is_admin: bool,
    is_backend: bool,
    has_profile: bool,
    is_named_agent: bool,
    /// Present in the member's channel: an active heartbeat (a live-state,
    /// unexpired status for that channel), for local and remote sessions
    /// alike. Relay membership persists across ordinary stops, so a named
    /// agent without one is a dormant member, not an active agent.
    #[serde(default)]
    is_live: bool,
}

impl MemberFacts {
    pub(crate) fn is_named_agent(self) -> bool {
        self.is_named_agent
    }

    pub(crate) fn is_live(self) -> bool {
        self.is_live
    }
}

/// Identity evidence shared by every member normalization in one capture.
#[derive(Default)]
pub(crate) struct MemberFactIndex {
    named: BTreeSet<String>,
    /// Per channel, the pubkeys with an active heartbeat (`Status::is_heartbeat`).
    heartbeats: BTreeMap<String, BTreeSet<String>>,
    backend: BTreeSet<String>,
}

impl MemberFactIndex {
    pub(crate) fn capture(store: &Store, local_backend: &str) -> Result<Self> {
        let statuses = store.list_status_sessions(None, None)?;
        let mut named = store
            .list_named_session_pubkeys()?
            .into_iter()
            .collect::<BTreeSet<_>>();
        named.extend(
            statuses
                .iter()
                .filter(|status| !status.slug.trim().is_empty())
                .map(|status| status.pubkey.clone()),
        );
        let mut heartbeats: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for status in statuses.iter().filter(|status| status.is_heartbeat()) {
            heartbeats
                .entry(status.channel_h.clone())
                .or_default()
                .insert(status.pubkey.clone());
        }
        let mut backend = store
            .list_backend_profiles()?
            .into_iter()
            .map(|profile| profile.pubkey)
            .collect::<BTreeSet<_>>();
        if !local_backend.is_empty() {
            backend.insert(local_backend.to_string());
        }
        Ok(Self {
            named,
            heartbeats,
            backend,
        })
    }

    pub(crate) fn normalize(&self, store: &Store, member: &ChannelMember) -> Result<MemberFacts> {
        let profile = store.get_profile(&member.pubkey)?;
        Ok(MemberFacts {
            is_admin: member.role == "admin",
            is_backend: self.backend.contains(&member.pubkey)
                || profile.as_ref().is_some_and(|profile| profile.is_backend),
            has_profile: profile.is_some(),
            is_named_agent: self.named.contains(&member.pubkey)
                || profile
                    .as_ref()
                    .is_some_and(|profile| !profile.agent_slug.trim().is_empty()),
            is_live: self
                .heartbeats
                .get(&member.channel_h)
                .is_some_and(|pubkeys| pubkeys.contains(&member.pubkey)),
        })
    }
}

/// Count live named non-admin agents. Dormant named members (stopped sessions
/// that keep relay membership) are not counted. A roster is unknowable until both relay role
/// snapshots hydrate, and any unresolved ordinary identity keeps it unknown.
pub(crate) fn count_agents(
    hydrated: bool,
    members: impl IntoIterator<Item = MemberFacts>,
) -> Option<usize> {
    if !hydrated {
        return None;
    }
    let mut count = 0;
    for member in members {
        if member.is_admin || member.is_backend {
            continue;
        }
        if member.is_named_agent {
            if member.is_live {
                count += 1;
            }
        } else if !member.has_profile {
            return None;
        }
    }
    Some(count)
}

#[cfg(test)]
mod tests;
