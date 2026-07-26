use std::collections::BTreeSet;

use anyhow::Result;

use crate::state::Store;

pub(super) struct Facts {
    named: BTreeSet<String>,
    backend: BTreeSet<String>,
}

pub(super) fn capture(store: &Store, local_backend: &str) -> Result<Facts> {
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
    Ok(Facts { named, backend })
}

pub(super) fn count(store: &Store, channel: &str, facts: &Facts) -> Result<Option<usize>> {
    if !store.has_channel_membership_snapshot(channel)? {
        return Ok(None);
    }
    let mut count = 0;
    for member in store.list_channel_members(channel)? {
        let profile = store.get_profile(&member.pubkey)?;
        let is_backend = facts.backend.contains(&member.pubkey)
            || profile.as_ref().is_some_and(|profile| profile.is_backend);
        let is_named_agent = facts.named.contains(&member.pubkey)
            || profile
                .as_ref()
                .is_some_and(|profile| !profile.agent_slug.trim().is_empty());
        match crate::agent_count::classify(
            &member.role,
            is_backend,
            profile.is_some(),
            is_named_agent,
        ) {
            crate::agent_count::MemberClass::Agent => count += 1,
            crate::agent_count::MemberClass::Unknown => return Ok(None),
            crate::agent_count::MemberClass::Ignore | crate::agent_count::MemberClass::Human => {}
        }
    }
    Ok(Some(count))
}
