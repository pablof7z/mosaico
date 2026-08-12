use std::collections::BTreeSet;

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
}

/// Identity evidence shared by every member normalization in one capture.
#[derive(Default)]
pub(crate) struct MemberFactIndex {
    named: BTreeSet<String>,
    backend: BTreeSet<String>,
}

impl MemberFactIndex {
    pub(crate) fn capture(store: &Store, local_backend: &str) -> Result<Self> {
        let mut named = store
            .list_sessions()?
            .into_iter()
            .filter(|session| !session.agent_slug.trim().is_empty())
            .map(|session| session.pubkey)
            .collect::<BTreeSet<_>>();
        named.extend(
            store
                .list_status_sessions(None, None)?
                .into_iter()
                .filter(|status| !status.slug.trim().is_empty())
                .map(|status| status.pubkey),
        );
        let mut backend = store
            .list_backend_profiles()?
            .into_iter()
            .map(|profile| profile.pubkey)
            .collect::<BTreeSet<_>>();
        if !local_backend.is_empty() {
            backend.insert(local_backend.to_string());
        }
        Ok(Self { named, backend })
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
        })
    }
}

/// Count named non-admin agents. A roster is unknowable until both relay role
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
            count += 1;
        } else if !member.has_profile {
            return None;
        }
    }
    Some(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Profile, TestRelayDelivery};

    fn profile(pubkey: &str, name: &str, slug: &str, agent_slug: &str) -> Profile {
        Profile {
            pubkey: pubkey.into(),
            name: name.into(),
            slug: slug.into(),
            agent_slug: agent_slug.into(),
            host: "host".into(),
            is_backend: false,
            agents: Vec::new(),
            workspaces: Vec::new(),
            updated_at: 1,
        }
    }

    fn facts(
        is_admin: bool,
        is_backend: bool,
        has_profile: bool,
        is_named_agent: bool,
    ) -> MemberFacts {
        MemberFacts {
            is_admin,
            is_backend,
            has_profile,
            is_named_agent,
        }
    }

    #[test]
    fn canonical_member_count_matrix_preserves_unknown_semantics() {
        let cases = [
            (
                "unhydrated",
                false,
                vec![facts(false, false, true, true)],
                None,
            ),
            (
                "backend",
                true,
                vec![facts(false, true, false, false)],
                Some(0),
            ),
            (
                "admin",
                true,
                vec![facts(true, false, false, true)],
                Some(0),
            ),
            (
                "named agent",
                true,
                vec![facts(false, false, false, true)],
                Some(1),
            ),
            (
                "known human with handle",
                true,
                vec![facts(false, false, true, false)],
                Some(0),
            ),
            (
                "known human with empty slug",
                true,
                vec![facts(false, false, true, false)],
                Some(0),
            ),
            (
                "unknown identity",
                true,
                vec![facts(false, false, false, false)],
                None,
            ),
        ];

        for (name, hydrated, members, expected) in cases {
            assert_eq!(count_agents(hydrated, members), expected, "{name}");
        }
    }

    #[test]
    fn normalized_store_matrix_is_shared_by_both_projection_surfaces() {
        let store = Store::open_memory().unwrap();
        store.install_test_nmp_relay_delivery(TestRelayDelivery::new().profiles([
            profile("agent", "Agent", "agent", "codex"),
            profile("human", "Human", "human", ""),
            profile("empty-human", "Human", "", ""),
        ]));
        let index = MemberFactIndex::capture(&store, "backend").unwrap();
        let member = |pubkey: &str, role: &str| ChannelMember {
            channel_h: "room".into(),
            pubkey: pubkey.into(),
            role: role.into(),
        };
        let cases = [
            (
                "backend recovery identity",
                member("backend", "member"),
                Some(0),
            ),
            ("unknown admin", member("admin", "admin"), Some(0)),
            ("named agent", member("agent", "member"), Some(1)),
            ("known human", member("human", "member"), Some(0)),
            (
                "known human with empty slug",
                member("empty-human", "member"),
                Some(0),
            ),
            ("unknown identity", member("unknown", "member"), None),
        ];

        for (name, member, expected) in cases {
            let normalized = index.normalize(&store, &member).unwrap();
            assert_eq!(count_agents(true, [normalized]), expected, "{name}");
            assert_eq!(count_agents(false, [normalized]), None, "{name} cold");
        }
    }
}
