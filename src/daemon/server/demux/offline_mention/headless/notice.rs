use crate::daemon::server::DaemonState;
use crate::daemon::tail_event::TailEvent;
use crate::domain::{AgentRef, ChatMessage};
use crate::fabric::provider::chat::OutboundChatRecord;
use crate::util::now_secs;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HeadlessOutcome {
    Exited(String),
    WaitFailed(String),
    WaitTaskFailed(String),
}

impl HeadlessOutcome {
    fn detail(&self) -> String {
        match self {
            HeadlessOutcome::Exited(status) => format!("process exited with {status}"),
            HeadlessOutcome::WaitFailed(error) => format!("wait failed: {error}"),
            HeadlessOutcome::WaitTaskFailed(error) => format!("wait task failed: {error}"),
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
    pub(super) exec_id: &'a str,
    pub(super) pid: i32,
    pub(super) outcome: &'a HeadlessOutcome,
    pub(super) log_path: &'a Path,
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
        mentioned_pubkey: None,
    };
    let record = OutboundChatRecord {
        from_session: None,
        channel_h: notice.channel.to_string(),
        body: body.clone(),
        mentioned_pubkey: None,
        mentioned_session: None,
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
        to: notice.agent_slug.to_string(),
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
    let subject = notice
        .session_id
        .map(|session| format!("session {session}"))
        .unwrap_or_else(|| format!("pid {}", notice.pid));
    format!(
        "tenex-edge: {} headless run for {subject} exited without publishing a chat reply ({}). exec={}; log={}",
        notice.agent_slug,
        notice.outcome.detail(),
        notice.exec_id,
        notice.log_path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn no_reply_notice_mentions_session_outcome_and_log() {
        let outcome = HeadlessOutcome::Exited("exit status: 0".to_string());
        let log_path = PathBuf::from("/tmp/exec-codex-1.log");
        let body = no_reply_notice_body(&NoReplyNotice {
            agent_slug: "codex",
            channel: "chan",
            session_id: Some("te-session"),
            exec_id: "exec-codex-1",
            pid: 4321,
            outcome: &outcome,
            log_path: &log_path,
        });

        assert!(body.contains("codex headless run for session te-session"));
        assert!(body.contains("without publishing a chat reply"));
        assert!(body.contains("process exited with exit status: 0"));
        assert!(body.contains("exec=exec-codex-1"));
        assert!(body.contains("log=/tmp/exec-codex-1.log"));
    }

    #[test]
    fn no_reply_notice_falls_back_to_pid_before_registration() {
        let outcome = HeadlessOutcome::WaitFailed("no child".to_string());
        let log_path = PathBuf::from("/tmp/exec-claude-1.log");
        let body = no_reply_notice_body(&NoReplyNotice {
            agent_slug: "claude",
            channel: "chan",
            session_id: None,
            exec_id: "exec-claude-1",
            pid: 9876,
            outcome: &outcome,
            log_path: &log_path,
        });

        assert!(body.contains("claude headless run for pid 9876"));
        assert!(body.contains("wait failed: no child"));
    }
}
