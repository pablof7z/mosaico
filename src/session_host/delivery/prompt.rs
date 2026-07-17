use crate::daemon::server::DaemonState;
use crate::session_host::transport::{EndpointRef, TransportImpl};
use crate::util::now_secs;
use anyhow::Result;
use std::sync::Arc;

struct PendingPrompt {
    text: String,
    chat_ids: Vec<String>,
    /// The most recent injected mention — the event an auto-reply threads to,
    /// the channel it publishes into, and the requester it p-tags.
    trigger_event_id: String,
    trigger_channel: String,
    trigger_from_pubkey: String,
}

async fn collect_pending_prompt(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    event_ids: &[String],
) -> Result<Option<PendingPrompt>> {
    let now = now_secs();
    let mut chat_rows =
        state.with_store(|s| s.claim_pending_event_ids_for_pubkey(event_ids, &rec.pubkey, now))?;
    if chat_rows.is_empty() {
        return Ok(None);
    }
    crate::profile::label_chat_senders(state, &mut chat_rows).await;

    let whitelisted = state.whitelisted_pubkeys().to_vec();
    let chat_ids: Vec<String> = chat_rows.iter().map(|row| row.event_id.clone()).collect();
    let rendered = state.with_store(|s| {
        crate::injection::render_terminal_mention(s, &chat_rows, &whitelisted, now)
    });
    let Some(text) = rendered else {
        if let Err(e) = state.with_store(|s| s.reenqueue_pending(&chat_ids, &rec.pubkey)) {
            tracing::error!(
                pubkey = %rec.pubkey,
                error = %e,
                "failed to re-enqueue claimed-but-unrendered inbox rows; mention may be lost"
            );
            state.emit_delivery_failure(
                &rec.channel_h,
                &rec.agent_slug,
                &rec.pubkey,
                format!("failed to re-enqueue claimed-but-unrendered inbox rows: {e:#}"),
            );
        }
        return Ok(None);
    };

    let trigger = chat_rows.last();
    let trigger_event_id = trigger.map(|r| r.event_id.clone()).unwrap_or_default();
    let trigger_channel = trigger
        .map(|r| r.channel_h.clone())
        .unwrap_or_else(|| rec.channel_h.clone());
    let trigger_from_pubkey = trigger.map(|r| r.from_pubkey.clone()).unwrap_or_default();
    Ok(Some(PendingPrompt {
        text,
        chat_ids,
        trigger_event_id,
        trigger_channel,
        trigger_from_pubkey,
    }))
}

pub(super) async fn inject_planned_messages(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    transport: &TransportImpl,
    endpoint_id: &str,
    event_ids: &[String],
) -> Result<bool> {
    let endpoint = EndpointRef {
        kind: transport.kind(),
        endpoint_id: endpoint_id.to_string(),
    };
    if !transport.is_live(&endpoint) {
        anyhow::bail!(
            "{} session {endpoint_id} is not live",
            transport.kind().as_str()
        );
    }
    let Some(prompt) = collect_pending_prompt(state, rec, event_ids).await? else {
        return Ok(false);
    };

    if let Err(e) = transport.deliver(&endpoint, &prompt.text, true).await {
        reenqueue_after_failure(state, rec, &prompt.chat_ids, "transport delivery");
        return Err(e);
    }
    finalize_injection(state, rec, &prompt)?;
    Ok(true)
}

/// Roll claimed inbox rows back to `pending` after a delivery failure so the
/// mention is retried rather than lost. Emits a delivery failure if the rollback
/// itself fails (the only way a mention truly leaks).
fn reenqueue_after_failure(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    chat_ids: &[String],
    what: &str,
) {
    if let Err(re) = state.with_store(|s| s.reenqueue_pending(chat_ids, &rec.pubkey)) {
        tracing::error!(
            pubkey = %rec.pubkey,
            error = %re,
            "failed to roll back claimed inbox rows after {what} failure; mention may be lost"
        );
        state.emit_delivery_failure(
            &rec.channel_h,
            &rec.agent_slug,
            &rec.pubkey,
            format!("failed to roll back claimed inbox rows after {what} failure: {re:#}"),
        );
    }
}

/// Post-delivery bookkeeping shared by the PTY and ACP injectors: flip the
/// delivered rows to `injected` (echo-suppression on PTY; fabric-context de-dup on
/// both transports — a mention handed to the agent as literal input must not
/// re-appear as fresh chat context), then arm a turn-end auto-reply.
fn finalize_injection(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    prompt: &PendingPrompt,
) -> Result<()> {
    if let Err(e) = state.with_store(|s| s.mark_injected_for_echo(&prompt.chat_ids, &rec.pubkey)) {
        tracing::error!(
            pubkey = %rec.pubkey,
            error = %e,
            "failed to mark injected inbox rows for echo suppression"
        );
        state.emit_delivery_failure(
            &rec.channel_h,
            &rec.agent_slug,
            &rec.pubkey,
            format!("failed to mark injected inbox rows for echo suppression: {e:#}"),
        );
        anyhow::bail!("failed to mark injected inbox rows for echo suppression: {e:#}");
    }
    // Arm a turn-end auto-reply so the channel still hears back if the agent
    // finishes the turn without publishing its own `channel send`.
    if !prompt.trigger_event_id.is_empty()
        && crate::daemon::server::auto_reply::should_arm_for_session(rec)
    {
        crate::daemon::server::auto_reply::arm(
            &rec.pubkey,
            &prompt.trigger_channel,
            &prompt.trigger_event_id,
            &prompt.trigger_from_pubkey,
        );
    }
    Ok(())
}
