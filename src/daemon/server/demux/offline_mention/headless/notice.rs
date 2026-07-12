use crate::daemon::server::DaemonState;
use crate::daemon::tail_event::TailEvent;
use crate::domain::{AgentRef, ChatMessage};
use crate::fabric::provider::chat::{OutboundChatRecipient, OutboundChatRecord};
use crate::util::now_secs;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HeadlessOutcome {
    Exited { status: String, success: bool },
    StartFailed(String),
    WaitFailed(String),
    WaitTaskFailed(String),
}

impl HeadlessOutcome {
    fn detail(&self) -> String {
        match self {
            HeadlessOutcome::Exited { status, .. } => format!("process exited with {status}"),
            HeadlessOutcome::StartFailed(error) => format!("start failed: {error}"),
            HeadlessOutcome::WaitFailed(error) => format!("wait failed: {error}"),
            HeadlessOutcome::WaitTaskFailed(error) => format!("wait task failed: {error}"),
        }
    }

    fn failed_to_start(&self) -> bool {
        match self {
            HeadlessOutcome::Exited { success, .. } => !success,
            HeadlessOutcome::StartFailed(_) => true,
            HeadlessOutcome::WaitFailed(_) | HeadlessOutcome::WaitTaskFailed(_) => false,
        }
    }
}

pub(super) fn session_published_reply_since(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    started_at: u64,
) -> bool {
    match state.with_store(|s| s.session_has_outbound_message_since(&rec.session_id, started_at)) {
        Ok(has_reply) => has_reply,
        Err(e) => {
            tracing::warn!(
                session = %rec.session_id,
                agent = %rec.agent_slug,
                channel = %rec.channel_h,
                error = %e,
                "failed to check headless reply state"
            );
            false
        }
    }
}

pub(super) struct NoReplyNotice<'a> {
    pub(super) agent_slug: &'a str,
    pub(super) channel: &'a str,
    pub(super) session_id: Option<&'a str>,
    pub(super) requester_pubkey: Option<&'a str>,
    pub(super) target_label: Option<&'a str>,
    pub(super) exec_id: &'a str,
    pub(super) outcome: &'a HeadlessOutcome,
}

pub(super) async fn publish_no_reply_notice(state: &Arc<DaemonState>, notice: NoReplyNotice<'_>) {
    let body = no_reply_notice_body(&notice);
    let keys = match state.management_keys() {
        Ok(keys) => keys,
        Err(e) => {
            emit_local_notice_failure(state, &notice, body);
            tracing::warn!(
                agent = %notice.agent_slug,
                channel = notice.channel,
                exec_id = notice.exec_id,
                error = %e,
                "headless no-reply notice publish skipped: missing management keys"
            );
            return;
        }
    };
    let now = now_secs();
    let from = format!("{} (tenex-edge)", state.host);
    let chat = ChatMessage {
        from: AgentRef::new(keys.public_key().to_hex(), from.clone()),
        channel: notice.channel.to_string(),
        body: body.clone(),
        mentioned_pubkeys: notice
            .requester_pubkey
            .map(str::to_string)
            .into_iter()
            .collect(),
    };
    let record = OutboundChatRecord {
        from_session: None,
        channel_h: notice.channel.to_string(),
        body: body.clone(),
        recipients: notice
            .requester_pubkey
            .map(|pubkey| OutboundChatRecipient::new(pubkey, None))
            .into_iter()
            .collect(),
        created_at: Some(now),
        direction: "outbound",
    };
    let published = match state
        .provider
        .publish_chat_checked(&chat, &keys, &record)
        .await
    {
        Ok(published) => published,
        Err(e) => {
            emit_local_notice_failure(state, &notice, body);
            tracing::warn!(
                agent = %notice.agent_slug,
                channel = notice.channel,
                exec_id = notice.exec_id,
                error = %format!("{e:#}"),
                "headless no-reply notice publish failed"
            );
            return;
        }
    };
    state.emit_tail(TailEvent::Msg {
        ts: published.created_at,
        channel: notice.channel.to_string(),
        from,
        from_session: None,
        to: notice
            .requester_pubkey
            .map(crate::util::pubkey_short)
            .unwrap_or_else(|| notice.agent_slug.to_string()),
        to_session: notice.session_id.map(str::to_string),
        body: body.chars().take(200).collect(),
    });
}

fn emit_local_notice_failure(state: &Arc<DaemonState>, notice: &NoReplyNotice<'_>, body: String) {
    state.emit_delivery_failure(
        notice.channel,
        notice.agent_slug,
        notice.session_id.unwrap_or(notice.exec_id),
        body,
    );
}

fn no_reply_notice_body(notice: &NoReplyNotice<'_>) -> String {
    let target = notice
        .target_label
        .filter(|label| !label.is_empty())
        .unwrap_or(notice.agent_slug);
    if notice.outcome.failed_to_start() {
        format!(
            "tenex-edge: agent {target} failed to start for your mention ({}).",
            notice.outcome.detail()
        )
    } else {
        format!(
            "tenex-edge: agent {target} exited without replying to your mention ({}).",
            notice.outcome.detail()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_reply_notice_mentions_target_and_omits_log() {
        let outcome = HeadlessOutcome::Exited {
            status: "exit status: 0".to_string(),
            success: true,
        };
        let body = no_reply_notice_body(&NoReplyNotice {
            agent_slug: "codex",
            channel: "chan",
            session_id: Some("te-session"),
            requester_pubkey: Some("pk-requester"),
            target_label: Some("flint-range-108@laptop"),
            exec_id: "exec-codex-1",
            outcome: &outcome,
        });

        assert!(body.contains("agent flint-range-108@laptop"));
        assert!(body.contains("exited without replying to your mention"));
        assert!(body.contains("process exited with exit status: 0"));
        assert!(!body.contains("exec=exec-codex-1"));
        assert!(!body.contains("/tmp/exec-codex-1.log"));
    }

    #[test]
    fn nonzero_exit_reports_failed_start() {
        let outcome = HeadlessOutcome::Exited {
            status: "exit status: 2".to_string(),
            success: false,
        };
        let body = no_reply_notice_body(&NoReplyNotice {
            agent_slug: "claude",
            channel: "chan",
            session_id: None,
            requester_pubkey: None,
            target_label: None,
            exec_id: "exec-claude-1",
            outcome: &outcome,
        });

        assert!(body.contains("agent claude"));
        assert!(body.contains("failed to start for your mention"));
        assert!(body.contains("process exited with exit status: 2"));
    }
}
