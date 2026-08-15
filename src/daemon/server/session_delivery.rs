//! Durable inbox delivery for extension-native harness sessions.
//!
//! The extension talks to the normal daemon UDS. A wait atomically leases only
//! its exact recipient inbox; acknowledgement follows native-harness acceptance
//! of the rendered custom message. Ambient channel observation stays in normal
//! turn context and is never consumed here.

use super::*;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const LEASE_SECS: u64 = 45;
mod registry;
pub(crate) use registry::extension_delivery_live;
pub(super) use registry::ActiveExtensionDeliveryRegistry;
use registry::ExtensionWaitGuard;

#[cfg(test)]
#[path = "session_delivery/tests.rs"]
mod tests;

#[derive(serde::Deserialize)]
struct WaitParams {
    timeout_secs: u64,
}

#[derive(serde::Deserialize)]
struct AckParams {
    lease_id: String,
    accepted: bool,
}

pub(super) async fn rpc_wait(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: WaitParams = serde_json::from_value(params.clone())?;
    if p.timeout_secs == 0 || p.timeout_secs > 60 {
        anyhow::bail!("session delivery timeout must be between 1 and 60 seconds");
    }
    let rec = resolve_extension_session(state, params)?;
    let now = now_secs();
    let registry = state.runtime.extension_delivery.clone();
    let became_live = {
        let mut registry = registry.lock().expect("extension-delivery mutex poisoned");
        let became_live = registry.touch(&rec, now);
        registry.begin_wait(&rec);
        became_live
    };
    if became_live {
        let presence = state.clone();
        let pubkey = rec.pubkey.clone();
        let generation = rec.runtime_generation;
        tokio::spawn(async move {
            super::presence::reconcile_generation(
                &presence,
                &pubkey,
                generation,
                "pi_extension_delivery_live",
                None,
            )
            .await;
        });
    }
    let _guard = ExtensionWaitGuard::new(registry.clone(), rec.clone());
    let mut rx = state.tail_subscribe();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(p.timeout_secs);
    let timeout = tokio::time::sleep_until(deadline);
    tokio::pin!(timeout);
    loop {
        if let Some(response) = lease_delivery(state, &registry, &rec).await? {
            return Ok(response);
        }
        tokio::select! {
            _ = &mut timeout => return Ok(serde_json::json!({"kind":"timeout"})),
            event = rx.recv() => match event {
                Ok(TailEvent::Msg { .. }) => {}
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    anyhow::bail!("session delivery stream closed");
                }
            }
        }
    }
}

pub(super) async fn rpc_ack(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: AckParams = serde_json::from_value(params.clone())?;
    if p.lease_id.trim().is_empty() {
        anyhow::bail!("session delivery acknowledgement requires lease_id");
    }
    let rec = resolve_extension_session(state, params)?;
    let lease = state
        .runtime
        .extension_delivery
        .lock()
        .expect("extension-delivery mutex poisoned")
        .take_lease(&rec, &p.lease_id)
        .context("unknown, expired, or foreign session delivery lease")?;
    let confirmed = state.with_store(|store| {
        store.acknowledge_extension_lease(&lease.event_ids, &rec.pubkey, p.accepted, now_secs())
    })?;
    if p.accepted && confirmed.len() != lease.event_ids.len() {
        anyhow::bail!("session delivery lease was superseded before acknowledgement");
    }
    if p.accepted {
        if let Some(turn) = lease.reminder_turn {
            state.record_coordination_reminder(&rec.pubkey, turn);
        }
        crate::daemon::server::turns::work_start_reaction::publish_for_started_events(
            state, &rec, &confirmed,
        );
    }
    Ok(serde_json::json!({
        "state": if p.accepted { "injected" } else { "requeued" },
        "event_ids": confirmed,
    }))
}

async fn lease_delivery(
    state: &Arc<DaemonState>,
    registry: &Arc<Mutex<ActiveExtensionDeliveryRegistry>>,
    rec: &crate::state::Session,
) -> Result<Option<serde_json::Value>> {
    let rows =
        state.with_store(|store| store.lease_pending_for_extension(&rec.pubkey, now_secs()))?;
    if rows.is_empty() {
        return Ok(None);
    }
    let event_ids = rows
        .iter()
        .map(|row| row.event_id.clone())
        .collect::<Vec<_>>();
    let Some(prompt) = crate::session_host::render_inbox_rows(state, rec, rows).await? else {
        state.with_store(|store| store.reenqueue_extension_lease_ids(&event_ids, &rec.pubkey))?;
        anyhow::bail!("could not render pending inbox delivery");
    };
    let lease_id = registry
        .lock()
        .expect("extension-delivery mutex poisoned")
        .insert_lease(
            rec,
            prompt.chat_ids.clone(),
            prompt.coordination_reminder_turn,
        );
    schedule_expiry(state.clone(), rec.clone(), lease_id.clone());
    Ok(Some(serde_json::json!({
        "kind": "delivery",
        "lease_id": lease_id,
        "message": {
            "custom_type": "mosaico.delivery",
            "content": prompt.text,
            "display": false,
            "details": { "event_ids": prompt.chat_ids },
        },
    })))
}

fn schedule_expiry(state: Arc<DaemonState>, rec: crate::state::Session, lease_id: String) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(LEASE_SECS)).await;
        let lease = state
            .runtime
            .extension_delivery
            .lock()
            .expect("extension-delivery mutex poisoned")
            .take_lease(&rec, &lease_id);
        let Some(lease) = lease else {
            return;
        };
        match state
            .with_store(|store| store.reenqueue_extension_lease_ids(&lease.event_ids, &rec.pubkey))
        {
            Ok(ids) if !ids.is_empty() => crate::session_host::ring_doorbells(state),
            Ok(_) => {}
            Err(error) => tracing::error!(%error, "failed to requeue expired extension delivery"),
        }
    });
}

fn resolve_extension_session(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<crate::state::Session> {
    if params.get("harness").and_then(|value| value.as_str()) != Some("pi") {
        anyhow::bail!("session delivery is available only to the Pi extension");
    }
    let rec = resolve_session_inner(
        state,
        &CallerAnchor::from_params(params),
        ResolveScope::Strict,
    )?;
    if rec.observed_harness != "pi" {
        anyhow::bail!("session delivery requires a Pi-native session");
    }
    if rec.admitted_transport == "pi-rpc" {
        anyhow::bail!("managed Pi RPC owns inbox delivery for this session");
    }
    Ok(rec)
}
