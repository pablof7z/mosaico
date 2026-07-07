use super::{scope, StoreReader, WhoRow, WhoSource};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet, HashSet};

pub(super) fn push_claim_rows(
    store: StoreReader<'_>,
    current_project: Option<&str>,
    now: u64,
    local_host: &str,
    rows: &mut Vec<WhoRow>,
    other_agents: &mut BTreeMap<String, BTreeSet<String>>,
) -> Result<()> {
    let live_sessions: HashSet<String> = rows.iter().map(|r| r.session_id.clone()).collect();
    let live_pubkeys: HashSet<String> = rows.iter().map(|r| r.pubkey.clone()).collect();
    for claim in store.list_active_session_claims(now)? {
        if live_sessions.contains(&claim.session_id) || live_pubkeys.contains(&claim.pubkey) {
            continue;
        }
        let scope = claim.channel_h.clone();
        if scope::is_archived_channel(store, &scope) {
            continue;
        }
        let slug = crate::identity::AgentInstance::from_parts(
            claim.agent_slug.clone(),
            claim.base_pubkey.clone(),
            claim.ordinal,
            claim.pubkey.clone(),
        )
        .display_slug();
        if current_project
            .map(|p| scope::scope_contains_channel(store, p, &scope))
            .unwrap_or(true)
        {
            rows.push(dormant_row(store, claim, slug, local_host, now));
        } else if scope::is_root_channel(store, &scope) {
            other_agents.entry(scope).or_default().insert(slug);
        }
    }
    Ok(())
}

fn dormant_row(
    store: StoreReader<'_>,
    claim: crate::state::session_claims::SessionClaim,
    slug: String,
    local_host: &str,
    now: u64,
) -> WhoRow {
    let title = store
        .get_session(&claim.session_id)
        .ok()
        .flatten()
        .map(|s| s.title)
        .unwrap_or_default();
    WhoRow {
        source: WhoSource::Local,
        fresh: false,
        slug,
        project: claim.channel_h.clone(),
        status: title,
        activity: String::new(),
        active: false,
        dormant: true,
        host: local_host.to_string(),
        session_id: claim.session_id,
        age_secs: Some(now.saturating_sub(claim.last_active_at)),
        rel_cwd: String::new(),
        remote: false,
        attachable: false,
        work_root: scope::work_root_for(store, &claim.channel_h),
        pubkey: claim.pubkey,
    }
}
