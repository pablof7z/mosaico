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
        let running = match s.list_running_sessions() {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e, "ring_doorbells: list_running_sessions failed — skipping doorbell scan this tick");
                return Vec::new();
            }
        };
        let active_pubkeys = running.iter().map(|rec| rec.pubkey.clone()).collect();
        prune_debounce(&active_pubkeys);
        running
    });

    for rec in sessions {
        let pubkey = rec.pubkey.clone();
        let pending = match state.with_store(|s| s.peek_pending_for_pubkey(&pubkey)) {
            Ok(pending) => pending,
            Err(e) => {
                tracing::error!(%pubkey, error = %e, "ring_doorbells: peek_pending_for_pubkey failed");
                state.emit_delivery_failure(
                    "",
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
        let mut pending_channels = pending
            .iter()
            .map(|row| row.channel_h.clone())
            .collect::<Vec<_>>();
        pending_channels.sort();
        pending_channels.dedup();

        let hosted = match state.with_store(|s| hosted_endpoint_for(s, &rec)) {
            Ok(hosted) => hosted,
            Err(e) => {
                tracing::error!(%pubkey, error = %e, "ring_doorbells: locator lookup failed — cannot resolve endpoint this tick");
                for channel in &pending_channels {
                    state.emit_delivery_failure(
                        channel,
                        &rec.agent_slug,
                        &pubkey,
                        format!("failed to resolve delivery endpoint: {e:#}"),
                    );
                }
                continue;
            }
        };
        let (endpoint_id, endpoint_live) = match &hosted {
            crate::session_host::transport::HostedEndpoint::Resolved {
                transport,
                endpoint,
            } => (
                Some(endpoint.endpoint_id.clone()),
                transport.is_live(endpoint),
            ),
            crate::session_host::transport::HostedEndpoint::Unhosted
            | crate::session_host::transport::HostedEndpoint::Unavailable { .. } => (None, false),
        };
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
                for channel in &pending_channels {
                    state.emit_delivery_failure(
                        channel,
                        &rec.agent_slug,
                        &pubkey,
                        format!("delivery reconciler failed: {e:#}"),
                    );
                }
                continue;
            }
        };
        if let crate::session_host::transport::HostedEndpoint::Resolved { transport, .. } = hosted {
            apply_delivery_effects(state, &rec, &transport, effects).await;
        }
    }
    Ok(())
}

async fn apply_delivery_effects(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    transport: &crate::session_host::transport::TransportImpl,
    effects: Vec<crate::reconcile::DeliveryEffect>,
) {
    for effect in effects {
        match effect {
            crate::reconcile::DeliveryEffect::Inject {
                pubkey,
                endpoint_id,
                event_ids,
            } => {
                let result =
                    inject_planned_messages(state, rec, transport, &endpoint_id, &event_ids).await;
                match result {
                    Ok(true) => {
                        record_message_injection(&pubkey);
                        if std::env::var("MOSAICO_DEBUG").is_ok() {
                            eprintln!(
                                "[{}] pending messages delivered to endpoint {endpoint_id} for {pubkey}",
                                transport.kind().as_str()
                            );
                        }
                    }
                    Ok(false) => {}
                    Err(e) => {
                        let channels = state
                            .with_store(|store| store.peek_pending_for_pubkey(&pubkey))
                            .unwrap_or_default()
                            .into_iter()
                            .map(|row| row.channel_h)
                            .collect::<std::collections::BTreeSet<_>>();
                        for channel in channels {
                            state.emit_delivery_failure(
                                &channel,
                                &rec.agent_slug,
                                &pubkey,
                                format!(
                                    "pending message delivery failed for {} endpoint {endpoint_id}: {e:#}",
                                    transport.kind().as_str()
                                ),
                            );
                        }
                    }
                }
            }
            crate::reconcile::DeliveryEffect::RetryAfter { pubkey, delay_secs } => {
                schedule_delivery_retry(state.clone(), pubkey, delay_secs)
            }
            crate::reconcile::DeliveryEffect::ClearDeadEndpoint { pubkey } => {
                let _ = state.with_store(|s| {
                    s.clear_runtime_locator_if_generation(
                        &pubkey,
                        transport.kind().locator_kind(),
                        rec.runtime_generation,
                    )
                });
            }
        }
    }
}

fn schedule_delivery_retry(state: Arc<DaemonState>, pubkey: String, delay_secs: u64) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(delay_secs.max(1))).await;
        if std::env::var("MOSAICO_DEBUG").is_ok() {
            eprintln!("[transport] retrying deferred delivery for {pubkey}");
        }
        ring_doorbells(state);
    });
}
