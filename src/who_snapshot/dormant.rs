use super::{scope, WhoRow, WhoSource};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet, HashSet};

pub(super) fn push_claim_rows(
    aggregation: &crate::who_aggregation::WhoAggregation,
    current_root: Option<&str>,
    now: u64,
    local_host: &str,
    rows: &mut Vec<WhoRow>,
    other_agents: &mut BTreeMap<String, BTreeSet<String>>,
) -> Result<()> {
    let live_pubkeys: HashSet<String> = rows.iter().map(|r| r.pubkey.clone()).collect();
    for claim in aggregation.claims.iter().cloned() {
        if live_pubkeys.contains(&claim.pubkey) {
            continue;
        }
        let scope = claim.channel_h.clone();
        if scope::is_archived_channel(aggregation, &scope) {
            continue;
        }
        let slug = aggregation
            .display_slug(&claim.pubkey)
            .unwrap_or_else(|| claim.agent_slug.clone());
        if current_root
            .map(|p| scope::scope_contains_channel(aggregation, p, &scope))
            .transpose()?
            .unwrap_or(true)
        {
            rows.push(dormant_row(aggregation, claim, slug, local_host, now)?);
        } else if scope::is_root_channel(aggregation, &scope) {
            other_agents.entry(scope).or_default().insert(slug);
        }
    }
    Ok(())
}

fn dormant_row(
    aggregation: &crate::who_aggregation::WhoAggregation,
    claim: crate::state::session_claims::SessionClaim,
    slug: String,
    local_host: &str,
    now: u64,
) -> Result<WhoRow> {
    let owner_host = claim.owner_host.trim();
    let host = if owner_host.is_empty() {
        local_host.to_string()
    } else {
        owner_host.to_string()
    };
    let remote = !owner_host.is_empty() && owner_host != local_host;
    let title = aggregation
        .session(&claim.pubkey)
        .map(|s| s.title.clone())
        .unwrap_or_default();
    let work_root = scope::work_root_for(aggregation, &claim.channel_h)?;
    let work_root_display = work_root.clone();
    Ok(WhoRow {
        source: WhoSource::Local,
        state: crate::session_state::SessionState::Offline,
        slug,
        channel: claim.channel_h.clone(),
        status: title,
        activity: String::new(),
        dormant: true,
        host,
        age_secs: Some(now.saturating_sub(claim.last_active_at)),
        rel_cwd: String::new(),
        remote,
        work_root,
        work_root_display,
        pubkey: claim.pubkey,
    })
}
