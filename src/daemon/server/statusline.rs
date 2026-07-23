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
    let host = state.host.clone();
    // Routing scope is the session's channel — the member count and is_member
    // check key on it so a `channel switch` (which repoints channel_h) is
    // reflected in the statusline without restarting.
    let scope = rec.channel_h.clone();
    // Issue #98: one authoritative agent-instance identity for label + membership.
    let instance = state.session_instance(&rec);
    state.with_store(|s| {
        let member_count = s.count_channel_members(&scope).unwrap_or(0);
        // Resolve the ordinal label (e.g. "claude1" for the second concurrent
        // Claude session) through the authoritative AgentInstance projection.
        let agent_label = instance.display_slug();
        let is_member = s
            .is_channel_member(&scope, &instance.pubkey)
            .unwrap_or(true);
        // State and title come straight off the local session row. Pure read: no
        // drains, no touches.
        let published = s.get_status(&rec.pubkey, &scope).ok().flatten();
        let presence = crate::session_presence::local(s, &rec, published.as_ref());
        // `channel_title` is the channel's human handle from the relay-authored
        // kind:39000 metadata cache (relay_channels `name`). The channel name is
        // set only at create/edit, so it is the durable display label for this
        // scope (`title` is carried separately for the live status segment).
        let channel_title = s
            .get_channel(&scope)
            .ok()
            .flatten()
            .map(|c| c.name)
            .unwrap_or_default();
        // `work_root` is the top-level root this channel belongs under.
        // This is the "Root" line in `who`, surfaced as `root-name`.
        let work_root = crate::daemon::workspace_path::WorkspacePathResolver::new(s)
            .root_for_channel(&scope)?;
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
            "channel": scope,
            "channel_title": channel_title,
            "member_count": member_count,
            "is_member": is_member,
            "state": presence.state,
            "state_since": presence.state_since,
            "title": presence.title,
            "activity": presence.activity,
            "pending": pending_json,
            "recent": recent_json,
        }))
    })
}
