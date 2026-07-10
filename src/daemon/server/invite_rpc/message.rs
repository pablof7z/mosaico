//! `channel add --message "..."`: once the target session is online, post a
//! kind:9 chat into the channel mentioning it — add + mention in one shot.
//!
//! Rather than re-implement chat publishing (mention resolution, p-tag, local
//! doorbell delivery), this synthesizes a `chat_write` call from the SAME caller
//! anchors the invite ran under, prefixing the body with the brought-online
//! session's `@agent/session` handle so `chat_write` p-tags it.

use crate::daemon::server::DaemonState;
use std::sync::Arc;

/// Publish the add-message. `label_with_host` is the online session's public
/// `agent/session` handle. Returns an error STRING on failure
/// rather than propagating: the membership add already succeeded, so a failed
/// courtesy message must degrade to a warning, never fail the whole `channel add`.
pub(super) async fn post_add_message(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
    channel_h: &str,
    label_with_host: &str,
    message: &str,
) -> Option<String> {
    let mut chat = params.clone();
    let Some(obj) = chat.as_object_mut() else {
        return Some("invite params were not an object".to_string());
    };
    obj.insert("channel".into(), serde_json::json!(channel_h));
    obj.insert(
        "message".into(),
        serde_json::json!(format!("@{label_with_host} {message}")),
    );
    // The mention prefix can push a short message over the soft cap; the operator
    // already opted into posting it, so never reject on length here.
    obj.insert("long_message".into(), serde_json::json!(true));
    match crate::daemon::server::chat_write::rpc_chat_write(state, &chat).await {
        Ok(_) => None,
        Err(e) => Some(format!("{e:#}")),
    }
}
