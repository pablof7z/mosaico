//! `who --expired`: stopped local sessions a user can list and then resume.
//!
//! A session row whose runtime is stopped is an expired
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
    /// Last lease observation, unix seconds (0 when never seen).
    pub(crate) last_seen: u64,
    /// Whether exact recovery authority remains (native resume or fresh launch).
    pub(crate) resumable: bool,
}

/// The stopped local sessions, newest first, each named by its public handle.
/// Reads [`Store::list_resumable_sessions`] and keeps only stopped rows.
pub(crate) fn load_expired_sessions(
    store: &Store,
    host: &str,
    limit: u32,
) -> Vec<ExpiredSessionRow> {
    store
        .list_resumable_sessions(limit)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| !s.is_running())
        .map(|s| ExpiredSessionRow {
            agent_slug: s.agent_slug,
            pubkey: s.pubkey.clone(),
            npub: crate::idref::npub(&s.pubkey).unwrap_or_default(),
            handle: store.handle_for_pubkey(&s.pubkey).ok().flatten(),
            host: host.to_string(),
            channel: channel_name(store, &s.channel_h),
            last_seen: s.last_seen,
            resumable: true,
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
        let pubkey = format!("pk-{ext}");
        store
            .reserve_hook_session_for_test(&RegisterSession {
                pubkey: pubkey.clone(),
                observed_harness: "claude-code".into(),
                agent_slug: "coder".into(),
                channel_h: channel.into(),
                child_pid: Some(7),
                transcript_path: None,
                now: 1_000,
            })
            .unwrap();
        store
            .put_session_locator(
                "claude-code",
                crate::state::LOCATOR_NATIVE_RESUME,
                &format!("resume-{ext}"),
                &pubkey,
                1_000,
            )
            .unwrap();
        pubkey
    }

    #[test]
    fn only_stopped_sessions_are_listed_by_permanent_pubkey() {
        let store = Store::open_memory().unwrap();
        store.upsert_channel("proj", "main", "", "", 1).unwrap();
        let running_id = register(&store, "running", "proj");
        let stopped_id = register(&store, "stopped", "proj");
        store
            .mark_runtime_stopped(&stopped_id, crate::state::StopReason::HeadlessExit, 50)
            .unwrap();

        let rows = load_expired_sessions(&store, "laptop", 50);
        assert_eq!(
            rows.len(),
            1,
            "only the stopped session is expired: {rows:?}"
        );
        let row = &rows[0];
        assert_eq!(row.agent_slug, "coder");
        assert_eq!(row.pubkey, "pk-stopped");
        assert!(row.handle.is_none());
        assert!(
            row.npub.is_empty(),
            "fixture pubkey is intentionally invalid"
        );
        assert_ne!(stopped_id, running_id);
        assert_eq!(row.host, "laptop");
        assert_eq!(row.channel, "main");
        assert!(row.resumable, "row retains exact recovery authority");
    }
}
