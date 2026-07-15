//! Background inbox scan and reconciler-effect application.

use super::*;

/// Scan live sessions for unread inbox rows and schedule transport-aware
/// delivery without blocking the caller that observed the message.
pub fn ring_doorbells(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        if let Err(e) = ring_doorbells_inner(&state).await {
            tracing::error!(error = %format!("{e:#}"), "ring_doorbells: doorbell scan failed");
        }
    });
}

async fn ring_doorbells_inner(state: &Arc<DaemonState>) -> Result<()> {
    let sessions: Vec<crate::state::Session> = state.with_store(|s| {
        let alive = match s.list_alive_sessions() {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e, "ring_doorbells: list_alive_sessions failed — skipping doorbell scan this tick");
                return Vec::new();
            }
        };
        let active_pubkeys = alive.iter().map(|rec| rec.pubkey.clone()).collect();
        prune_debounce(&active_pubkeys);
        alive
    });

    for rec in sessions {
        let pubkey = rec.pubkey.clone();
        let pending = match state.with_store(|s| s.peek_pending_for_pubkey(&pubkey)) {
            Ok(pending) => pending,
            Err(e) => {
                tracing::error!(%pubkey, error = %e, "ring_doorbells: peek_pending_for_pubkey failed");
                state.emit_delivery_failure(
                    &rec.channel_h,
                    &rec.agent_slug,
                    &pubkey,
                    format!("failed to read pending inbox for doorbell scan: {e:#}"),
                );
                continue;
            }
        };
        if pending.is_empty() {
            continue;
        }

        let kind = match transport_kind_for_slug(&rec.agent_slug) {
            Ok(kind) => kind,
            Err(e) => {
                state.emit_delivery_failure(
                    &rec.channel_h,
                    &rec.agent_slug,
                    &pubkey,
                    format!("failed to resolve configured transport: {e:#}"),
                );
                continue;
            }
        };
        let endpoint_id = match state.with_store(|s| endpoint_id_for(s, &pubkey, kind)) {
            Ok(endpoint_id) => endpoint_id,
            Err(e) => {
                tracing::error!(%pubkey, error = %e, "ring_doorbells: locator lookup failed — cannot resolve endpoint this tick");
                state.emit_delivery_failure(
                    &rec.channel_h,
                    &rec.agent_slug,
                    &pubkey,
                    format!("failed to resolve delivery endpoint: {e:#}"),
                );
                continue;
            }
        };

        let endpoint_live = endpoint_id
            .as_deref()
            .is_some_and(|id| endpoint_is_live(kind, id));
        let fact = delivery_scan_fact(
            &rec,
            pending.into_iter().map(|row| row.event_id).collect(),
            endpoint_id,
            endpoint_live,
            false,
        );
        let effects = match state.drive_delivery_scan("doorbell", fact) {
            Ok(effects) => effects,
            Err(e) => {
                state.emit_delivery_failure(
                    &rec.channel_h,
                    &rec.agent_slug,
                    &pubkey,
                    format!("delivery reconciler failed: {e:#}"),
                );
                continue;
            }
        };
        apply_delivery_effects(state, &rec, kind, effects).await;
    }
    Ok(())
}

async fn apply_delivery_effects(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    kind: TransportKind,
    effects: Vec<crate::reconcile::DeliveryEffect>,
) {
    for effect in effects {
        match effect {
            crate::reconcile::DeliveryEffect::Inject {
                pubkey,
                endpoint_id,
                event_ids,
            } => {
                let result = match kind {
                    TransportKind::Pty => {
                        inject_planned_messages_pty(state, rec, &endpoint_id, &event_ids).await
                    }
                    TransportKind::Acp => {
                        inject_planned_messages_acp(state, rec, &endpoint_id, &event_ids).await
                    }
                };
                match result {
                    Ok(true) => {
                        record_message_injection(&pubkey);
                        if std::env::var("MOSAICO_DEBUG").is_ok() {
                            eprintln!(
                                "[{}] pending messages delivered to endpoint {endpoint_id} for {pubkey}",
                                kind.as_str()
                            );
                        }
                    }
                    Ok(false) => {}
                    Err(e) => state.emit_delivery_failure(
                        &rec.channel_h,
                        &rec.agent_slug,
                        &pubkey,
                        format!(
                            "pending message delivery failed for {} endpoint {endpoint_id}: {e:#}",
                            kind.as_str()
                        ),
                    ),
                }
            }
            crate::reconcile::DeliveryEffect::RetryAfter { pubkey, delay_secs } => {
                schedule_delivery_retry(state.clone(), pubkey, delay_secs)
            }
            crate::reconcile::DeliveryEffect::ClearDeadEndpoint { pubkey } => {
                let _ = state.with_store(|s| s.clear_locator_kind(&pubkey, locator_kind(kind)));
            }
        }
    }
}

fn schedule_delivery_retry(state: Arc<DaemonState>, pubkey: String, delay_secs: u64) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(delay_secs.max(1))).await;
        if std::env::var("MOSAICO_DEBUG").is_ok() {
            eprintln!("[pty] retrying deferred delivery for {pubkey}");
        }
        ring_doorbells(state);
    });
}
