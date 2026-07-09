use super::*;

pub(in crate::daemon::server) const STATUSLINE_RECENT_SECS: u64 = 30;

#[derive(serde::Deserialize, Default)]
pub(in crate::daemon::server) struct StatuslineParams {
    #[serde(default)]
    pub(in crate::daemon::server) session: Option<String>,
}

/// `statusline`: everything the host's status bar renders, in one pure-read RPC.
/// Like `turn_check`, this is called constantly by the harness, so it must
/// NEVER write to state.db (no drains, no touches) — peeks only.
pub(in crate::daemon::server) fn rpc_statusline(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: StatuslineParams = serde_json::from_value(params.clone()).unwrap_or_default();
    // Session ID is the only locator needed. Fail open (empty bar) when it is
    // absent or stale — the next session_start reassert will refresh @te_session
    // on the host integration and the bar recovers on the next refresh tick.
    let session_id = p.session.as_deref().filter(|s| !s.is_empty());
    let rec = match session_id {
        Some(id) => match state.with_store(|s| s.get_session(id)).ok().flatten() {
            Some(r) => r,
            None => {
                // ID present but not in the DB — stale @te_session (e.g. after a
                // DB restore). Return a visible error instead of an empty bar.
                return Ok(serde_json::json!({ "error": "stale" }));
            }
        },
        None => return Ok(serde_json::json!({})),
    };
    let now = now_secs();
    let host = state.host.clone();
    // Routing scope is the session's channel — the member count and is_member
    // check key on it so a `channels switch` (which repoints channel_h) is
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
        // Busy + title + live activity come straight off the local session row
        // (the pre-publish draft the distiller maintains). Pure read: no drains,
        // no touches. The statusline shows the activity line (the live "doing
        // now" signal), not the persistent title.
        let working = rec.working;
        let title = rec.title.clone();
        let activity = rec.activity.clone();
        // `channel_title` is the channel's human handle from the relay-authored
        // kind:39000 metadata cache (relay_channels `name`). The channel name is
        // set only at create/edit now — never from the distilled session title —
        // so it is the durable display label for this scope (the distilled
        // `title` is carried separately for the live status segment).
        let channel_title = s
            .get_channel(&scope)
            .ok()
            .flatten()
            .map(|c| c.name)
            .unwrap_or_default();
        // `work_root` is the top-level root this channel belongs under.
        // This is the "Root" line in `who`, surfaced as `root-name`.
        let work_root = s
            .root_channel_of(&scope)
            .ok()
            .flatten()
            .unwrap_or_else(|| scope.clone());
        let pending_chat = s
            .peek_pending_for_session(&rec.session_id)
            .unwrap_or_default();
        let recent_since = now.saturating_sub(STATUSLINE_RECENT_SECS);
        let recent_chat = s
            .recently_delivered_for_session(&rec.session_id, recent_since)
            .unwrap_or_default();
        let mut pending_json = chat_rows_to_json(s, &pending_chat);
        sort_message_json(&mut pending_json);
        let mut recent_json = chat_rows_to_json(s, &recent_chat);
        sort_message_json(&mut recent_json);
        Ok(serde_json::json!({
            "agent": agent_label,
            "host": host,
            "session_id": rec.session_id,
            "work_root": work_root,
            "channel": scope,
            "channel_title": channel_title,
            "member_count": member_count,
            "is_member": is_member,
            "working": working,
            "title": title,
            "activity": activity,
            "pending": pending_json,
            "recent": recent_json,
            "distill_error": serde_json::Value::Null,
        }))
    })
}
