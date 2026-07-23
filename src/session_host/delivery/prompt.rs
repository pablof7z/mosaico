use crate::daemon::server::DaemonState;
use crate::session_host::transport::{DeliveryCompletion, EndpointRef, TransportImpl};
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

    let completion = match transport.deliver(&endpoint, &prompt.text, true).await {
        Ok(completion) => completion,
        Err(error) => {
            reenqueue_after_failure(state, rec, &prompt.chat_ids, "transport delivery");
            return Err(error);
        }
    };
    finalize_injection(state, rec, &prompt)?;
    track_managed_turn(state, rec, &prompt.chat_ids, completion).await?;
    Ok(true)
}

/// RPC transports own their turn boundary, unlike PTY transports whose native
/// hooks project it. Start the durable turn only after the inbox rows are
/// committed as injected, then close it from the exact RPC completion signal.
/// This makes the resulting idle deadline mean "ten minutes since real work
/// finished" and atomically releases the injected-message eviction fence.
async fn track_managed_turn(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    event_ids: &[String],
    completion: DeliveryCompletion,
) -> Result<()> {
    let completion = match completion {
        DeliveryCompletion::ExternallyObserved => return Ok(()),
        DeliveryCompletion::Managed(completion) => completion,
        DeliveryCompletion::ManagedSteer(accepted) => {
            let state = state.clone();
            let rec = rec.clone();
            let event_ids = event_ids.to_vec();
            tokio::spawn(async move {
                match accepted.await {
                    Ok(Ok(())) => {
                        crate::daemon::server::turns::work_start_reaction::publish_for_started_events(
                            &state, &rec, &event_ids,
                        );
                    }
                    Ok(Err(error)) => tracing::warn!(
                        session = %rec.pubkey,
                        %error,
                        "app-server steer was not accepted; work-start reaction skipped"
                    ),
                    Err(_) => tracing::warn!(
                        session = %rec.pubkey,
                        "app-server steer confirmation was dropped; work-start reaction skipped"
                    ),
                }
            });
            return Ok(());
        }
    };
    let started_at = now_secs();
    let started = state.with_store(|store| {
        store.apply_session_turn_started(&rec.pubkey, rec.runtime_generation, started_at, None)
    })?;
    if !started {
        anyhow::bail!(
            "RPC turn started after session {} generation {} stopped",
            rec.pubkey,
            rec.runtime_generation
        );
    }
    crate::daemon::server::presence::reconcile_generation(
        state,
        &rec.pubkey,
        rec.runtime_generation,
        "managed_turn_started",
    )
    .await;
    crate::daemon::server::turns::work_start_reaction::publish_for_started_events(
        state, rec, event_ids,
    );

    let state = state.clone();
    let pubkey = rec.pubkey.clone();
    let generation = rec.runtime_generation;
    tokio::spawn(async move {
        match completion.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!(
                session = %pubkey,
                error = %format!("{error:#}"),
                "managed RPC turn ended with an error"
            ),
            Err(_) => tracing::warn!(
                session = %pubkey,
                "managed RPC turn completion sender was dropped"
            ),
        }
        match state
            .with_store(|store| store.apply_session_turn_ended(&pubkey, generation, now_secs()))
        {
            Ok(true) => {
                crate::daemon::server::presence::reconcile_generation(
                    &state,
                    &pubkey,
                    generation,
                    "managed_turn_ended",
                )
                .await;
                crate::session_host::ring_doorbells(state)
            }
            Ok(false) => tracing::debug!(
                session = %pubkey,
                generation,
                "managed RPC completion was superseded by a lifecycle edge"
            ),
            Err(error) => tracing::error!(
                session = %pubkey,
                generation,
                error = %error,
                "failed to project managed RPC turn completion"
            ),
        }
    });
    Ok(())
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
    if let Err(e) =
        state.with_store(|s| s.mark_injected_for_echo(&prompt.chat_ids, &rec.pubkey, now_secs()))
    {
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
