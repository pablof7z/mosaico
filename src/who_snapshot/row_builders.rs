use super::{scope::work_root_for, WhoRow, WhoSource};
use crate::state::{Session, Status, StoreReader};

/// Build a local-session row. Title/activity/busy prefer the agent's own status.
pub(super) fn local_row(store: StoreReader<'_>, s: &Session, local_host: &str, now: u64) -> WhoRow {
    let instance = local_instance(store, s);
    let live = store
        .get_status(&instance.pubkey, &s.channel_h)
        .ok()
        .flatten()
        .filter(|st| st.expiration == 0 || st.expiration >= now);
    let (title, activity, busy) = match live {
        Some(st) => (st.title, live_activity(st.busy, st.activity), st.busy),
        None => (
            s.title.clone(),
            live_activity(s.working, s.activity.clone()),
            s.working,
        ),
    };
    let work_root = work_root_for(store, &s.channel_h);
    WhoRow {
        source: WhoSource::Local,
        fresh: now.saturating_sub(s.last_seen) <= crate::session::STATUS_TTL_SECS,
        slug: instance.display_slug(),
        channel: s.channel_h.clone(),
        status: title,
        activity,
        active: busy,
        dormant: false,
        host: local_host.to_string(),
        age_secs: Some(now.saturating_sub(s.last_seen)),
        rel_cwd: String::new(),
        remote: false,
        attachable: false,
        work_root_display: work_root.clone(),
        work_root,
        pubkey: instance.pubkey,
    }
}

pub(super) fn local_instance(
    store: StoreReader<'_>,
    s: &Session,
) -> crate::identity::SessionIdentity {
    store
        .session_identity(&s.pubkey)
        .expect("session identity lookup failed")
        .expect("live session is missing its identity projection")
}

/// Build a peer row from relay-confirmed status; unknown host is treated local.
pub(super) fn peer_row(store: StoreReader<'_>, st: &Status, local_host: &str, now: u64) -> WhoRow {
    let host = store
        .get_profile(&st.pubkey)
        .ok()
        .flatten()
        .map(|p| p.host)
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| local_host.to_string());
    let work_root = work_root_for(store, &st.channel_h);
    WhoRow {
        source: WhoSource::Peer,
        fresh: true,
        slug: peer_slug(store, st),
        channel: st.channel_h.clone(),
        status: st.title.clone(),
        activity: live_activity(st.busy, st.activity.clone()),
        active: st.busy,
        dormant: false,
        remote: host.trim() != local_host,
        host,
        age_secs: Some(now.saturating_sub(st.last_seen)),
        rel_cwd: String::new(),
        attachable: false,
        work_root_display: work_root.clone(),
        work_root,
        pubkey: st.pubkey.clone(),
    }
}

pub(super) fn peer_slug(store: StoreReader<'_>, st: &Status) -> String {
    if !st.slug.is_empty() {
        return st.slug.clone();
    }
    store
        .resolve_slug_for_pubkey(&st.pubkey)
        .ok()
        .flatten()
        .unwrap_or_else(|| crate::util::pubkey_short(&st.pubkey))
}

fn live_activity(active: bool, activity: String) -> String {
    if active {
        activity
    } else {
        String::new()
    }
}
