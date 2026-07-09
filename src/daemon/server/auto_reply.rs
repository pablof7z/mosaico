//! Auto-publish an agent's final response when a pty-injected turn ends without
//! an explicit `chat write`.
//!
//! A hosted agent receives a kind:9 mention typed into its PTY, runs its turn,
//! and is expected to reply by invoking `tenex-edge chat write`. Some agents
//! finish the turn by printing a final answer to their own transcript and stop,
//! publishing nothing — so from the channel's perspective they never responded.
//!
//! This module closes that gap:
//!   1. [`arm`] records the un-answered mention when it is injected into the PTY
//!      (channel + triggering event id, for reply threading).
//!   2. [`note_published`] cancels it the moment the agent publishes itself.
//!   3. On turn end, [`take`] returns any still-pending entry and
//!      [`publish_last_response`] posts the transcript's last assistant text as
//!      the reply, threaded to the triggering event via an `e` tag.
//!
//! State is process-global and in-memory (mirroring the delivery debounce): the
//! daemon is a single process, and a restart at worst drops one pending
//! auto-reply — far simpler than a durable schema for ephemeral turn state.

use super::*;
use crate::fabric::provider::chat::OutboundChatRecord;
use std::sync::OnceLock;

#[cfg(test)]
mod tests;

/// A pty-injected kind:9 the agent has not yet replied to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::server) struct PendingAutoReply {
    channel_h: String,
    trigger_event_id: String,
}

#[derive(Default)]
struct AutoReplyTracker {
    pending: HashMap<String, PendingAutoReply>,
}

impl AutoReplyTracker {
    fn arm(&mut self, session_id: &str, channel_h: &str, trigger_event_id: &str) {
        self.pending.insert(
            session_id.to_string(),
            PendingAutoReply {
                channel_h: channel_h.to_string(),
                trigger_event_id: trigger_event_id.to_string(),
            },
        );
    }

    fn note_published(&mut self, session_id: &str) {
        self.pending.remove(session_id);
    }

    fn take(&mut self, session_id: &str) -> Option<PendingAutoReply> {
        self.pending.remove(session_id)
    }
}

static TRACKER: OnceLock<Mutex<AutoReplyTracker>> = OnceLock::new();

fn tracker() -> &'static Mutex<AutoReplyTracker> {
    TRACKER.get_or_init(|| Mutex::new(AutoReplyTracker::default()))
}

fn lock() -> std::sync::MutexGuard<'static, AutoReplyTracker> {
    tracker().lock().expect("auto-reply mutex poisoned")
}

/// Record that a kind:9 was injected into `session_id`'s PTY and owes a reply.
/// A later arm supersedes an earlier un-answered one: the newest mention is what
/// an auto-reply should thread to. Called from the pty delivery path.
pub(crate) fn arm(session_id: &str, channel_h: &str, trigger_event_id: &str) {
    lock().arm(session_id, channel_h, trigger_event_id);
}

/// The agent published a reply itself this turn — drop any pending auto-reply so
/// the turn end does not double-post.
pub(in crate::daemon::server) fn note_published(session_id: &str) {
    lock().note_published(session_id);
}

/// Take the pending auto-reply for a finished turn, if the agent never
/// published one. `None` for turns that were not pty-injected (never armed) or
/// that the agent already answered.
pub(in crate::daemon::server) fn take(session_id: &str) -> Option<PendingAutoReply> {
    lock().take(session_id)
}

/// Publish the session's last assistant transcript text as its reply, threaded
/// to the triggering event. No-op (with a warning on failure) when there is no
/// transcript or no assistant text to publish.
pub(in crate::daemon::server) async fn publish_last_response(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    pending: PendingAutoReply,
) {
    let Some(path) = rec.transcript_path.clone() else {
        return;
    };
    let body = crate::transcript::read_last_assistant_text(
        std::path::Path::new(&path),
        crate::util::CHAT_WRITE_CHAR_LIMIT,
    );
    let Some(body) = body.filter(|b| !b.trim().is_empty()) else {
        return;
    };
    if let Err(e) = do_publish(state, rec, &pending, &body).await {
        tracing::warn!(
            session_id = %rec.session_id,
            channel = %pending.channel_h,
            error = %format!("{e:#}"),
            "auto-reply: failed to publish agent's last response"
        );
    }
}

async fn do_publish(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    pending: &PendingAutoReply,
    body: &str,
) -> Result<()> {
    let instance = state.session_instance(rec);
    let keys = state.session_signing_keys(&rec.session_id)?;
    let chat = ChatMessage {
        from: instance.agent_ref(),
        project: pending.channel_h.clone(),
        body: body.to_string(),
        mentioned_pubkey: None,
    };
    let published = state
        .provider
        .publish_chat_reply_checked(
            &chat,
            &pending.trigger_event_id,
            &keys,
            &OutboundChatRecord {
                from_session: Some(rec.session_id.clone()),
                channel_h: pending.channel_h.clone(),
                body: body.to_string(),
                mentioned_pubkey: None,
                mentioned_session: None,
                created_at: Some(now_secs()),
                direction: "outbound",
            },
        )
        .await?;
    state.emit_tail(TailEvent::Msg {
        ts: published.created_at,
        project: pending.channel_h.clone(),
        from: instance.display_slug(),
        from_session: Some(rec.session_id.clone()),
        to: "project-chat".to_string(),
        to_session: None,
        body: body.chars().take(200).collect(),
    });
    Ok(())
}
