use crate::daemon::server::DaemonState;
use crate::util::now_secs;
use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;

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
    let mut chat_rows = state
        .with_store(|s| s.claim_pending_event_ids_for_session(event_ids, &rec.session_id, now))?;
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
        if let Err(e) = state.with_store(|s| s.reenqueue_pending(&chat_ids, &rec.session_id)) {
            tracing::error!(
                session_id = %rec.session_id,
                error = %e,
                "failed to re-enqueue claimed-but-unrendered inbox rows; mention may be lost"
            );
            state.emit_delivery_failure(
                &rec.channel_h,
                &rec.agent_slug,
                &rec.session_id,
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

pub(super) async fn inject_planned_messages_pty(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    pty_id: &str,
    event_ids: &[String],
) -> Result<bool> {
    if !crate::pty::is_live(pty_id) {
        anyhow::bail!("pty session {pty_id} is not live");
    }
    let Some(prompt) = collect_pending_prompt(state, rec, event_ids).await? else {
        return Ok(false);
    };

    // Bracketed-paste the rendered mention WITHOUT submitting, so terminal echo
    // and the follow-up newline stay under our control.
    if let Err(e) = crate::pty::inject(pty_id, &prompt.text, true, false) {
        reenqueue_after_failure(state, rec, &prompt.chat_ids, "pty inject");
        return Err(e);
    }
    finalize_injection(state, rec, &prompt)?;
    // The mention is now in the agent's PTY. Submit it after a short beat so the
    // paste settles before the newline (PTY-only: ACP submits inline).
    tokio::time::sleep(Duration::from_millis(200)).await;
    crate::pty::inject(pty_id, "", false, true)?;
    Ok(true)
}

/// ACP counterpart of [`inject_planned_messages_pty`]: deliver the rendered
/// mention over JSON-RPC (`AcpTransport::deliver`, fire-and-forget) rather than a
/// PTY bracketed paste. `endpoint_id` is the ACP endpoint recorded under the
/// session's `pty_session` alias. ACP has no terminal echo, so there is no
/// paste/newline split and no echo round-trip to await — the render is submitted
/// as a fresh turn in one call.
pub(super) async fn inject_planned_messages_acp(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    endpoint_id: &str,
    event_ids: &[String],
) -> Result<bool> {
    use crate::session_host::transport::{
        AcpTransport, EndpointRef, SessionTransport, TransportKind,
    };
    let ep = EndpointRef {
        kind: TransportKind::Acp,
        endpoint_id: endpoint_id.to_string(),
    };
    if !AcpTransport.is_live(&ep) {
        anyhow::bail!("acp session {endpoint_id} is not live");
    }
    let Some(prompt) = collect_pending_prompt(state, rec, event_ids).await? else {
        return Ok(false);
    };

    // Submit the rendered mention as a fresh turn (submit=true). `deliver` returns
    // promptly (the turn runs in a detached task); it does not block for the turn.
    if let Err(e) = AcpTransport.deliver(&ep, &prompt.text, true).await {
        reenqueue_after_failure(state, rec, &prompt.chat_ids, "acp deliver");
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
    if let Err(re) = state.with_store(|s| s.reenqueue_pending(chat_ids, &rec.session_id)) {
        tracing::error!(
            session_id = %rec.session_id,
            error = %re,
            "failed to roll back claimed inbox rows after {what} failure; mention may be lost"
        );
        state.emit_delivery_failure(
            &rec.channel_h,
            &rec.agent_slug,
            &rec.session_id,
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
    if let Err(e) =
        state.with_store(|s| s.mark_injected_for_echo(&prompt.chat_ids, &rec.session_id))
    {
        tracing::error!(
            session_id = %rec.session_id,
            error = %e,
            "failed to mark injected inbox rows for echo suppression"
        );
        state.emit_delivery_failure(
            &rec.channel_h,
            &rec.agent_slug,
            &rec.session_id,
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
            &rec.session_id,
            &prompt.trigger_channel,
            &prompt.trigger_event_id,
            &prompt.trigger_from_pubkey,
        );
    }
    Ok(())
}
