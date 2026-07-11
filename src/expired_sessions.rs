//! `who --expired`: the dead/old local sessions a user can list and then resume.
//!
//! A session row that is no longer `alive` (its process exited) is an expired
//! session. Its npub is the permanent resume selector; a current leased handle
//! is optional presentation data.

use crate::state::Store;

/// One expired local session, permanently named by npub with an optional lease.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ExpiredSessionRow {
    /// Stable agent slug from the session row.
    #[serde(default)]
    pub(crate) agent_slug: String,
    pub(crate) pubkey: String,
    pub(crate) npub: String,
    pub(crate) handle: Option<String>,
    /// The daemon host these sessions belong to (they are always local).
    pub(crate) host: String,
    /// Human channel name (falls back to the raw channel id when unnamed).
    pub(crate) channel: String,
    /// Last heartbeat, unix seconds (0 when never seen).
    pub(crate) last_seen: u64,
    /// Whether a resume token is present — the session can be reconstituted.
    pub(crate) resumable: bool,
}

/// The not-currently-live local sessions (process exited), newest first, each
/// named by its public handle. Reads [`Store::list_resumable_sessions`] (alive OR
/// dead, capped) and keeps only the dead rows.
pub(crate) fn load_expired_sessions(
    store: &Store,
    host: &str,
    limit: u32,
) -> Vec<ExpiredSessionRow> {
    store
        .list_resumable_sessions(limit)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| !s.alive)
        .map(|s| ExpiredSessionRow {
            agent_slug: s.agent_slug,
            pubkey: s.agent_pubkey.clone(),
            npub: crate::idref::npub(&s.agent_pubkey).unwrap_or_default(),
            handle: store.handle_for_pubkey(&s.agent_pubkey).ok().flatten(),
            host: host.to_string(),
            channel: channel_name(store, &s.channel_h),
            last_seen: s.last_seen,
            resumable: !s.resume_id.is_empty(),
        })
        .collect()
}

fn channel_name(store: &Store, channel_h: &str) -> String {
    store
        .get_channel(channel_h)
        .ok()
        .flatten()
        .and_then(|c| c.human_name().map(str::to_string))
        .unwrap_or_else(|| channel_h.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RegisterSession;

    fn register(store: &Store, ext: &str, channel: &str) -> String {
        store
            .register_session(&RegisterSession {
                harness: "claude-code".into(),
                external_id_kind: "harness_session".into(),
                external_id: ext.into(),
                agent_pubkey: format!("pk-{ext}"),
                agent_slug: "coder".into(),
                channel_h: channel.into(),
                child_pid: Some(7),
                transcript_path: None,
                resume_id: format!("resume-{ext}"),
                now: 1_000,
            })
            .unwrap()
    }

    #[test]
    fn only_dead_sessions_are_listed_by_permanent_pubkey() {
        let store = Store::open_memory().unwrap();
        store.upsert_channel("proj", "main", "", "", 1).unwrap();
        let alive_id = register(&store, "alive", "proj");
        let dead_id = register(&store, "dead", "proj");
        store.mark_dead(&dead_id).unwrap();

        let rows = load_expired_sessions(&store, "laptop", 50);
        assert_eq!(rows.len(), 1, "only the dead session is expired: {rows:?}");
        let row = &rows[0];
        assert_eq!(row.agent_slug, "coder");
        assert_eq!(row.pubkey, "pk-dead");
        assert!(row.handle.is_none());
        assert!(
            row.npub.is_empty(),
            "fixture pubkey is intentionally invalid"
        );
        assert_ne!(dead_id, alive_id);
        assert_eq!(row.host, "laptop");
        assert_eq!(row.channel, "main");
        assert!(row.resumable, "row carries a resume token");
    }
}
