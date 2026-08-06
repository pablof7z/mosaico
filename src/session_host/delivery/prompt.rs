use crate::daemon::server::DaemonState;
use crate::session_host::transport::{EndpointRef, TransportImpl};
use crate::util::now_secs;
use anyhow::Result;
use std::sync::Arc;

#[path = "prompt/managed_turn.rs"]
mod managed_turn;
#[cfg(test)]
#[path = "prompt/tests.rs"]
mod tests;

struct PendingPrompt {
    text: String,
    chat_ids: Vec<String>,
    channels: Vec<String>,
    coordination_reminder_turn: Option<u64>,
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
    let mut channels = chat_rows
        .iter()
        .map(|row| row.channel_h.clone())
        .collect::<Vec<_>>();
    channels.sort();
    channels.dedup();
    let reminder_turn = if rec.is_working() {
        rec.turn_count.max(1)
    } else {
        rec.turn_count.saturating_add(1).max(1)
    };
    let unresolved =
        state.with_store(|s| crate::injection::has_unresolved_terminal_mention(s, &chat_rows));
    let show_guide = unresolved && state.coordination_reminder_due(&rec.pubkey, reminder_turn);
    let rendered = state.with_store(|s| {
        crate::injection::render_terminal_mention(s, &chat_rows, &whitelisted, now, show_guide)
    });
    let Some(text) = rendered else {
        if let Err(e) = state.with_store(|s| s.reenqueue_pending(&chat_ids, &rec.pubkey)) {
            tracing::error!(
                pubkey = %rec.pubkey,
                error = %e,
                "failed to re-enqueue claimed-but-unrendered inbox rows; mention may be lost"
            );
            emit_delivery_failures(
                state,
                rec,
                &channels,
                format!("failed to re-enqueue claimed-but-unrendered inbox rows: {e:#}"),
            );
        }
        return Ok(None);
    };

    Ok(Some(PendingPrompt {
        text,
        chat_ids,
        channels,
        coordination_reminder_turn: show_guide.then_some(reminder_turn),
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

    let delivered = transport.deliver(&endpoint, &prompt.text, true).await;
    let completion = match finish_delivery(
        state,
        &rec.pubkey,
        prompt.coordination_reminder_turn,
        delivered,
    ) {
        Ok(completion) => completion,
        Err(error) => {
            reenqueue_after_failure(
                state,
                rec,
                &prompt.chat_ids,
                &prompt.channels,
                "transport delivery",
            );
            return Err(error);
        }
    };
    // PTY write is best-effort: only mark `submitted` and wait for the
    // user-prompt-submit hook to corroborate before `injected`. RPC transports
    // own acceptance and can mark injected immediately.
    finalize_injection(state, rec, &prompt, &completion)?;
    if matches!(
        completion,
        crate::session_host::transport::DeliveryCompletion::ExternallyObserved
    ) {
        schedule_submitted_reenqueue(state.clone(), rec.pubkey.clone());
    }
    managed_turn::track(
        state,
        rec,
        &prompt.chat_ids,
        crate::state::NativeTurnDeliveryKind::InboxEvent,
        prompt.chat_ids.last().map(String::as_str).unwrap_or(""),
        completion,
    )
    .await?;
    Ok(true)
}

/// If the harness never fires user-prompt-submit for a PTY paste, roll the
/// submission back so doorbell/hook delivery can retry.
const PTY_SUBMIT_CONFIRM_TIMEOUT_SECS: u64 = 30;

fn schedule_submitted_reenqueue(state: Arc<DaemonState>, pubkey: String) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(
            PTY_SUBMIT_CONFIRM_TIMEOUT_SECS,
        ))
        .await;
        let before = now_secs().saturating_sub(PTY_SUBMIT_CONFIRM_TIMEOUT_SECS.saturating_sub(1));
        let requeued = match state.with_store(|s| s.reenqueue_stale_submitted(&pubkey, before)) {
            Ok(ids) => ids,
            Err(e) => {
                tracing::error!(
                    %pubkey,
                    error = %e,
                    "failed to reenqueue stale PTY submissions"
                );
                return;
            }
        };
        if !requeued.is_empty() {
            tracing::info!(
                %pubkey,
                count = requeued.len(),
                "requeued unconfirmed PTY submissions after timeout"
            );
            crate::session_host::ring_doorbells(state);
        }
    });
}

fn finish_delivery<T>(
    state: &Arc<DaemonState>,
    pubkey: &str,
    reminder_turn: Option<u64>,
    delivered: Result<T>,
) -> Result<T> {
    let completion = delivered?;
    if let Some(turn_count) = reminder_turn {
        state.record_coordination_reminder(pubkey, turn_count);
    }
    Ok(completion)
}

pub(super) async fn track_spawn_prompt(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    completion: crate::session_host::transport::DeliveryCompletion,
) -> Result<()> {
    managed_turn::track(
        state,
        rec,
        &[],
        crate::state::NativeTurnDeliveryKind::SpawnPrompt,
        "",
        completion,
    )
    .await
}

/// Roll claimed inbox rows back to `pending` after a delivery failure so the
/// mention is retried rather than lost. Emits a delivery failure if the rollback
/// itself fails (the only way a mention truly leaks).
fn reenqueue_after_failure(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    chat_ids: &[String],
    channels: &[String],
    what: &str,
) {
    if let Err(re) = state.with_store(|s| s.reenqueue_pending(chat_ids, &rec.pubkey)) {
        tracing::error!(
            pubkey = %rec.pubkey,
            error = %re,
            "failed to roll back claimed inbox rows after {what} failure; mention may be lost"
        );
        emit_delivery_failures(
            state,
            rec,
            channels,
            format!("failed to roll back claimed inbox rows after {what} failure: {re:#}"),
        );
    }
}

/// Post-delivery bookkeeping: RPC transports that own acceptance mark
/// `injected` immediately. PTY only records `submitted` until the harness
/// user-prompt hook corroborates the paste.
fn finalize_injection(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    prompt: &PendingPrompt,
    completion: &crate::session_host::transport::DeliveryCompletion,
) -> Result<()> {
    let now = now_secs();
    let result = match completion {
        crate::session_host::transport::DeliveryCompletion::ExternallyObserved => state
            .with_store(|s| {
                s.mark_submitted_for_prompt_confirm(&prompt.chat_ids, &rec.pubkey, now)
            }),
        crate::session_host::transport::DeliveryCompletion::Managed { .. }
        | crate::session_host::transport::DeliveryCompletion::ManagedSteer(_) => state
            .with_store(|s| s.mark_injected_for_echo(&prompt.chat_ids, &rec.pubkey, now)),
    };
    if let Err(e) = result {
        tracing::error!(
            pubkey = %rec.pubkey,
            error = %e,
            "failed to record post-delivery inbox state"
        );
        emit_delivery_failures(
            state,
            rec,
            &prompt.channels,
            format!("failed to record post-delivery inbox state: {e:#}"),
        );
        anyhow::bail!("failed to record post-delivery inbox state: {e:#}");
    }
    Ok(())
}

fn emit_delivery_failures(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    channels: &[String],
    detail: String,
) {
    for channel in channels {
        state.emit_delivery_failure(channel, &rec.agent_slug, &rec.pubkey, detail.clone());
    }
}
