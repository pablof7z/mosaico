use crate::daemon::server::DaemonState;
use crate::domain::{AgentRef, ChatMessage};
use std::sync::Arc;

pub(super) async fn publish_start_failure_notice(
    state: &Arc<DaemonState>,
    agent_slug: &str,
    target_label: &str,
    channel: &str,
    requester_pubkey: Option<&str>,
    detail: &str,
) {
    let target = if target_label.is_empty() {
        agent_slug
    } else {
        target_label
    };
    let body = format!("mosaico: agent {target} failed to start for your mention ({detail}).");
    let keys = match state.management_keys() {
        Ok(keys) => keys,
        Err(error) => {
            state.emit_delivery_failure(channel, agent_slug, "spawn", body);
            tracing::warn!(agent = agent_slug, channel, %error, "start-failure notice skipped");
            return;
        }
    };
    let from = format!("{} (mosaico)", state.host());
    let chat = ChatMessage {
        from: AgentRef::new(keys.public_key().to_hex(), from.clone()),
        channel: channel.to_string(),
        body: body.clone(),
        mentioned_pubkeys: requester_pubkey.map(str::to_string).into_iter().collect(),
        attachments: Vec::new(),
    };
    match state.provider().publish_chat_checked(&chat, &keys).await {
        Ok(_) => {}
        Err(error) => {
            state.emit_delivery_failure(channel, agent_slug, "spawn", body);
            tracing::warn!(agent = agent_slug, channel, error = %format!("{error:#}"), "start-failure notice publish failed");
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn failure_notice_names_the_target() {
        let target = "flint-range-108@laptop";
        let body = format!("mosaico: agent {target} failed to start for your mention (boom).");
        assert!(body.contains(target));
        assert!(body.contains("failed to start"));
    }
}
