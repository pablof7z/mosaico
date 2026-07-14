//! Auto-publish an agent's final response when a pty-injected turn ends without
//! an explicit `channel send`.
//!
//! A hosted agent receives a kind:9 mention typed into its PTY, runs its turn,
//! and is expected to reply by invoking `tenex-edge channel send`. Some agents
//! finish the turn by printing a final answer to their own transcript and stop,
//! publishing nothing — so from the channel's perspective they never responded.
//!
//! This module closes that gap:
//!   1. [`arm`] records the un-answered mention when it is injected into the PTY
//!      (channel, triggering event id, and requester pubkey).
//!   2. [`note_explicit_publish`] cancels it the moment the agent publishes
//!      itself and blocks future arming for that live daemon process.
//!   3. On turn end, [`take`] returns any still-pending entry and
//!      [`publish_last_response`] posts the transcript's last assistant text as
//!      the reply, threaded to the triggering event via an `e` tag and
//!      addressed back to the requester via a `p` tag.
//!
//! State is process-global and in-memory (mirroring the delivery debounce): the
//! daemon is a single process, and a restart at worst drops one pending
//! auto-reply — far simpler than a durable schema for ephemeral turn state.

use super::*;
use crate::fabric::provider::chat::{OutboundChatRecipient, OutboundChatRecord};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

#[cfg(test)]
mod tests;

/// A pty-injected kind:9 the agent has not yet replied to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::server) struct PendingAutoReply {
    channel_h: String,
    trigger_event_id: String,
    requester_pubkey: String,
}

#[derive(Default)]
struct AutoReplyTracker {
    pending: HashMap<String, PendingAutoReply>,
    explicit_publishers: HashSet<String>,
}

impl AutoReplyTracker {
    fn arm(
        &mut self,
        session_id: &str,
        channel_h: &str,
        trigger_event_id: &str,
        requester_pubkey: &str,
    ) -> bool {
        if self.explicit_publishers.contains(session_id) {
            return false;
        }
        self.pending.insert(
            session_id.to_string(),
            PendingAutoReply {
                channel_h: channel_h.to_string(),
                trigger_event_id: trigger_event_id.to_string(),
                requester_pubkey: requester_pubkey.to_string(),
            },
        );
        true
    }

    fn note_explicit_publish(&mut self, session_id: &str) {
        self.explicit_publishers.insert(session_id.to_string());
        self.pending.remove(session_id);
    }

    fn has_explicit_publish(&self, session_id: &str) -> bool {
        self.explicit_publishers.contains(session_id)
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
/// an auto-reply should thread to and p-tag back. Called from the pty delivery path.
pub(crate) fn arm(
    session_id: &str,
    channel_h: &str,
    trigger_event_id: &str,
    requester_pubkey: &str,
) -> bool {
    lock().arm(session_id, channel_h, trigger_event_id, requester_pubkey)
}

/// True when auto-reply may be armed for this session. The durable session marker
/// survives daemon restarts; the in-memory marker covers the gap if the store
/// update failed after a successful explicit publish.
pub(crate) fn should_arm_for_session(rec: &crate::state::Session) -> bool {
    rec.explicit_chat_published_at == 0 && !lock().has_explicit_publish(&rec.session_id)
}

/// The agent published through an explicit channel command. Drop any pending
/// auto-reply so this turn does not double-post, and block future arming.
pub(in crate::daemon::server) fn note_explicit_publish(session_id: &str) {
    lock().note_explicit_publish(session_id);
}

/// Take the pending auto-reply for a finished turn, if the agent never
/// published one. `None` for turns that were not pty-injected (never armed) or
/// that the agent already answered.
pub(in crate::daemon::server) fn take(session_id: &str) -> Option<PendingAutoReply> {
    lock().take(session_id)
}

/// Publish the session's last assistant transcript text as its reply, threaded
/// to the triggering event and p-tagged back to the requester. No-op (with a
/// warning on failure) when there is no transcript or no assistant text to publish.
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
        crate::util::CHANNEL_MESSAGE_CHAR_LIMIT,
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
    let keys = state.session_signing_keys(&rec.agent_pubkey)?;
    let recipients = if pending.requester_pubkey.trim().is_empty() {
        Vec::new()
    } else {
        vec![OutboundChatRecipient::new(pending.requester_pubkey.clone())]
    };
    let chat = ChatMessage {
        from: instance.agent_ref(),
        channel: pending.channel_h.clone(),
        body: body.to_string(),
        mentioned_pubkeys: recipients.iter().map(|r| r.pubkey.clone()).collect(),
    };
    let published = state
        .provider
        .publish_chat_reply_checked(
            &chat,
            &pending.trigger_event_id,
            &keys,
            &OutboundChatRecord {
                channel_h: pending.channel_h.clone(),
                body: body.to_string(),
                recipients,
                created_at: Some(now_secs()),
                direction: "outbound",
            },
        )
        .await?;
    state.emit_tail(TailEvent::Msg {
        ts: published.created_at,
        channel: pending.channel_h.clone(),
        from: instance.display_slug(),
        to: "channel-chat".to_string(),
        body: body.chars().take(200).collect(),
    });
    Ok(())
}
