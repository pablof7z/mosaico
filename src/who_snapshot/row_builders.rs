use super::{scope::work_root_for, WhoRow, WhoSource};
use crate::state::{Session, Status};
use anyhow::Result;

/// Build a local-session row. Relay-confirmed agent-supplied status wins when present.
pub(super) fn local_row(
    aggregation: &crate::who_aggregation::WhoAggregation,
    s: &Session,
    local_host: &str,
    now: u64,
) -> Result<WhoRow> {
    let instance = local_instance(aggregation, s);
    let presence = aggregation.local_session_presence(s);
    let work_root = work_root_for(aggregation, &s.channel_h)?;
    Ok(WhoRow {
        source: WhoSource::Local,
        state: presence.state,
        slug: instance.display_slug(),
        channel: s.channel_h.clone(),
        status: presence.title,
        activity: presence.activity,
        dormant: false,
        host: local_host.to_string(),
        age_secs: Some(now.saturating_sub(presence.observed_at)),
        rel_cwd: String::new(),
        remote: false,
        work_root_display: work_root.clone(),
        work_root,
        pubkey: instance.pubkey,
    })
}

pub(super) fn local_instance(
    aggregation: &crate::who_aggregation::WhoAggregation,
    s: &Session,
) -> crate::identity::SessionIdentity {
    aggregation
        .session_identity(&s.pubkey)
        .cloned()
        .expect("live session is missing its identity projection")
}

/// Build a peer row from relay-confirmed status; unknown host is treated local.
pub(super) fn peer_row(
    aggregation: &crate::who_aggregation::WhoAggregation,
    st: &Status,
    local_host: &str,
    now: u64,
) -> Result<WhoRow> {
    let host = aggregation
        .profile(&st.pubkey)
        .map(|p| p.host.clone())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| local_host.to_string());
    let work_root = work_root_for(aggregation, &st.channel_h)?;
    let presence = crate::session_presence::remote(st, now);
    Ok(WhoRow {
        source: WhoSource::Peer,
        state: presence.state,
        slug: peer_slug(aggregation, st),
        channel: st.channel_h.clone(),
        status: presence.title,
        activity: presence.activity,
        dormant: false,
        remote: host.trim() != local_host,
        host,
        age_secs: Some(now.saturating_sub(presence.observed_at)),
        rel_cwd: String::new(),
        work_root_display: work_root.clone(),
        work_root,
        pubkey: st.pubkey.clone(),
    })
}

pub(super) fn peer_slug(
    aggregation: &crate::who_aggregation::WhoAggregation,
    st: &Status,
) -> String {
    if !st.slug.is_empty() {
        return st.slug.clone();
    }
    aggregation
        .display_slug(&st.pubkey)
        .unwrap_or_else(|| crate::util::pubkey_short(&st.pubkey))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_peer_is_offline_without_stale_activity() {
        let store = crate::state::Store::open_memory().unwrap();
        store.upsert_channel("root", "root", "", "", 1).unwrap();
        let status = Status {
            pubkey: "peer".into(),
            channel_h: "root".into(),
            slug: "reviewer".into(),
            title: "Reviewing".into(),
            activity: "stale live activity".into(),
            state: crate::session_state::SessionState::Working,
            state_since: 90,
            last_seen: 90,
            updated_at: 90,
            expiration: 100,
        };
        let aggregation = crate::who_aggregation::WhoAggregation::load(&store, 101).unwrap();
        let row = peer_row(&aggregation, &status, "laptop", 101).unwrap();
        assert_eq!(row.state, crate::session_state::SessionState::Offline);
        assert!(row.activity.is_empty());
    }
}
