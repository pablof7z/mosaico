use super::*;

pub(in crate::daemon::server) const STATUSLINE_RECENT_SECS: u64 = 30;

/// `statusline`: everything the host's status bar renders, in one pure-read RPC.
/// Like `turn_check`, this is called constantly by the harness, so it must
/// NEVER write to state.db (no drains, no touches) — peeks only.
pub(in crate::daemon::server) fn rpc_statusline(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let anchor = CallerAnchor::from_params(params);
    if anchor.explicit.is_none()
        && anchor.pty_session.is_none()
        && anchor.harness_session.is_none()
        && anchor.watch_pid.is_none()
    {
        return Ok(serde_json::json!({}));
    }
    let rec = match resolve_session(state, &anchor) {
        Ok(rec) => rec,
        Err(_) => return Ok(serde_json::json!({ "error": "stale" })),
    };
    let now = now_secs();
    let host = state.host().clone();
    // Issue #98: one authoritative agent-instance identity for label + membership.
    let instance = state.session_instance(&rec);
    state.with_store(|s| {
        // Resolve the ordinal label (e.g. "claude1" for the second concurrent
        // Claude session) through the authoritative AgentInstance projection.
        let agent_label = instance.display_slug();
        // State and title come straight off the local session row. Pure read: no
        // drains, no touches.
        let routes = s.list_session_routes(&rec.pubkey).unwrap_or_default();
        let published = routes
            .iter()
            .find_map(|(channel, _)| s.get_status(&rec.pubkey, channel).ok().flatten());
        let presence = crate::session_presence::local(s, &rec, published.as_ref());
        let channels = routes
            .iter()
            .filter_map(|(channel, _)| {
                super::channel_resolve::channel_reference_for(s, channel).ok()
            })
            .collect::<Vec<_>>();
        let work_root = if rec.work_root.is_empty() {
            String::new()
        } else {
            super::channel_resolve::channel_reference_for(s, &rec.work_root).unwrap_or_default()
        };
        let pending_chat = s.peek_pending_for_pubkey(&rec.pubkey).unwrap_or_default();
        let recent_since = now.saturating_sub(STATUSLINE_RECENT_SECS);
        let recent_chat = s
            .recently_delivered_for_pubkey(&rec.pubkey, recent_since)
            .unwrap_or_default();
        let mut pending_json = chat_rows_to_json(s, &pending_chat);
        sort_message_json(&mut pending_json);
        let mut recent_json = chat_rows_to_json(s, &recent_chat);
        sort_message_json(&mut recent_json);
        Ok(serde_json::json!({
            "agent": agent_label,
            "host": host,
            "work_root": work_root,
            "channels": channels,
            "state": presence.state,
            "state_since": presence.state_since,
            "title": presence.title,
            "activity": presence.activity,
            "pending": pending_json,
            "recent": recent_json,
        }))
    })
}
