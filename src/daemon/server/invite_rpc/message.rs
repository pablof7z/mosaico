//! `channel add --message "..."`: once the target session is online, post a
//! kind:9 chat into the channel mentioning it — add + mention in one shot.
//!
//! Rather than re-implement chat publishing (mention resolution, p-tag, local
//! doorbell delivery), this synthesizes a `channel_send` call from the SAME caller
//! anchors the invite ran under and passes the brought-online session's
//! `sessionCode-agent` handle through the canonical tag field.

use crate::daemon::server::DaemonState;
use std::sync::Arc;

/// Publish the add-message. `session_handle` is the online session's public
/// `sessionCode-agent` handle. Returns an error STRING on failure
/// rather than propagating: the membership add already succeeded, so a failed
/// courtesy message must degrade to a warning, never fail the whole `channel add`.
pub(super) async fn post_add_message(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
    channel_h: &str,
    session_handle: &str,
    message: &str,
) -> Option<String> {
    let mut chat = crate::daemon::server::channel_send::caller_params(params);
    let Some(obj) = chat.as_object_mut() else {
        return Some("invite params were not an object".to_string());
    };
    obj.insert("channel".into(), serde_json::json!(channel_h));
    obj.insert("message".into(), serde_json::json!(message));
    obj.insert("tags".into(), serde_json::json!([session_handle]));
    match crate::daemon::server::channel_send::rpc_channel_send(state, &chat).await {
        Ok(_) => None,
        Err(e) => Some(format!("{e:#}")),
    }
}
