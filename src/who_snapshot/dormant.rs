use super::{scope, WhoRow, WhoSource};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet, HashSet};

pub(super) fn push_stopped_rows(
    aggregation: &crate::who_aggregation::WhoAggregation,
    current_root: Option<&str>,
    now: u64,
    local_host: &str,
    rows: &mut Vec<WhoRow>,
    other_agents: &mut BTreeMap<String, BTreeSet<String>>,
) -> Result<()> {
    let live_pubkeys: HashSet<String> = rows.iter().map(|row| row.pubkey.clone()).collect();
    for standing in &aggregation.stopped_standing {
        if live_pubkeys.contains(&standing.pubkey)
            || scope::is_archived_channel(aggregation, &standing.channel_h)
        {
            continue;
        }
        let Some(session) = aggregation.session(&standing.pubkey) else {
            continue;
        };
        let slug = aggregation
            .display_slug(&standing.pubkey)
            .unwrap_or_else(|| session.agent_slug.clone());
        if current_root
            .map(|root| scope::scope_contains_channel(aggregation, root, &standing.channel_h))
            .transpose()?
            .unwrap_or(true)
        {
            rows.push(stopped_row(
                aggregation,
                session,
                &standing.channel_h,
                slug,
                local_host,
                now,
            )?);
        } else if scope::is_root_channel(aggregation, &standing.channel_h) {
            other_agents
                .entry(standing.channel_h.clone())
                .or_default()
                .insert(slug);
        }
    }
    Ok(())
}

fn stopped_row(
    aggregation: &crate::who_aggregation::WhoAggregation,
    session: &crate::state::Session,
    channel: &str,
    slug: String,
    local_host: &str,
    now: u64,
) -> Result<WhoRow> {
    let work_root = scope::work_root_for(aggregation, channel)?;
    Ok(WhoRow {
        source: WhoSource::Local,
        state: crate::session_state::SessionState::Offline,
        slug,
        channel: channel.to_string(),
        status: session.title.clone(),
        activity: String::new(),
        dormant: true,
        host: local_host.to_string(),
        age_secs: Some(now.saturating_sub(session.stopped_at)),
        rel_cwd: String::new(),
        remote: false,
        work_root_display: work_root.clone(),
        work_root,
        pubkey: session.pubkey.clone(),
    })
}
