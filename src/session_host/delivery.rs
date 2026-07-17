use crate::daemon::server::DaemonState;
use crate::util::now_secs;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

#[path = "delivery/doorbell.rs"]
mod doorbell;
#[path = "delivery/output_mode.rs"]
mod output_mode;
mod prompt;
use crate::session_host::transport::{transport_for_kind, EndpointRef, TransportKind};
pub use doorbell::ring_doorbells;
#[cfg(test)]
pub(crate) use output_mode::headless_for_endpoint;
pub(crate) use output_mode::session_is_headless;
use prompt::inject_planned_messages;

#[cfg(test)]
#[path = "delivery/tests.rs"]
mod delivery_tests;

fn locator_kind(kind: TransportKind) -> &'static str {
    match kind {
        TransportKind::Pty => crate::state::LOCATOR_PTY,
        TransportKind::Acp => crate::state::LOCATOR_ACP,
    }
}

fn endpoint_id_for(
    store: &crate::state::Store,
    pubkey: &str,
    harness: &str,
    kind: TransportKind,
) -> Result<Option<String>> {
    Ok(store
        .locator_for_session(pubkey, harness, locator_kind(kind))?
        .map(|locator| locator.locator_value))
}

/// Liveness of a session's typed transport endpoint.
fn endpoint_is_live(kind: TransportKind, endpoint_id: &str) -> bool {
    let transport = transport_for_kind(kind);
    transport.is_live(&EndpointRef {
        kind,
        endpoint_id: endpoint_id.to_string(),
    })
}

fn delivery_endpoint_for(
    store: &crate::state::Store,
    pubkey: &str,
) -> Result<Option<(EndpointRef, bool)>> {
    let endpoints = store
        .locators_for_pubkey(pubkey)?
        .into_iter()
        .filter_map(|locator| {
            let kind = match locator.locator_kind.as_str() {
                crate::state::LOCATOR_PTY => TransportKind::Pty,
                crate::state::LOCATOR_ACP => TransportKind::Acp,
                _ => return None,
            };
            Some(EndpointRef {
                kind,
                endpoint_id: locator.locator_value,
            })
        })
        .collect::<Vec<_>>();
    Ok(endpoints
        .iter()
        .find(|endpoint| endpoint_is_live(endpoint.kind, &endpoint.endpoint_id))
        .cloned()
        .map(|endpoint| (endpoint, true))
        .or_else(|| {
            endpoints
                .into_iter()
                .next()
                .map(|endpoint| (endpoint, false))
        }))
}

/// Whether `session` has a live daemon-owned delivery endpoint.
pub(crate) fn session_has_live_delivery_path(
    store: &crate::state::Store,
    session: &crate::state::Session,
) -> bool {
    let kind = match TransportKind::parse(&session.admitted_transport) {
        Some(kind) => kind,
        None => return false,
    };
    let locator = match store.locator_for_session(
        &session.pubkey,
        &session.observed_harness,
        locator_kind(kind),
    ) {
        Ok(locator) => locator,
        Err(e) => {
            tracing::error!(
                pubkey = %session.pubkey,
                error = %e,
                "delivery endpoint check: locator lookup failed; assuming unavailable"
            );
            return false;
        }
    };
    locator.is_some_and(|locator| endpoint_is_live(kind, &locator.locator_value))
}

/// Don't re-inject into the same session within this window (seconds).
const MESSAGE_INJECT_DEBOUNCE_SECS: u64 = 20;

static DEBOUNCE: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
fn debounce() -> &'static Mutex<HashMap<String, u64>> {
    DEBOUNCE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn last_message_injection(pubkey: &str) -> Option<u64> {
    debounce().lock().unwrap().get(pubkey).copied()
}

fn record_message_injection(pubkey: &str) {
    debounce()
        .lock()
        .unwrap()
        .insert(pubkey.to_string(), now_secs());
}

fn prune_debounce(active_pubkeys: &HashSet<String>) {
    let now = now_secs();
    debounce().lock().unwrap().retain(|pubkey, last| {
        active_pubkeys.contains(pubkey) && now.saturating_sub(*last) < MESSAGE_INJECT_DEBOUNCE_SECS
    });
}

/// Deliver a fresh session's opening prompt over whichever transport hosts it:
/// ACP via a JSON-RPC `deliver` (submit=true → a fresh turn), PTY via the
/// bracketed-paste spawn inject. The ACP child lives in the daemon registry, so
/// this MUST run in the daemon (the caller that spawned it). Failures are logged,
/// not propagated: the session is already live and can still receive mentions via
/// the doorbell path.
pub async fn deliver_spawn_prompt(kind: TransportKind, endpoint_id: &str, text: &str) {
    let transport = transport_for_kind(kind);
    tokio::time::sleep(transport.opening_delivery_delay()).await;
    let endpoint = EndpointRef {
        kind,
        endpoint_id: endpoint_id.to_string(),
    };
    if let Err(e) = transport.deliver(&endpoint, text, true).await {
        tracing::warn!(
            endpoint = %endpoint_id,
            transport = kind.as_str(),
            error = %format!("{e:#}"),
            "failed to deliver spawn prompt"
        );
    }
}

pub async fn inject_pending_messages_pty(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    endpoint_id: &str,
) -> Result<bool> {
    let pending = state.with_store(|s| s.peek_pending_for_pubkey(&rec.pubkey))?;
    if pending.is_empty() {
        return Ok(false);
    };
    let transport = transport_for_kind(TransportKind::Pty);
    let endpoint = EndpointRef {
        kind: TransportKind::Pty,
        endpoint_id: endpoint_id.to_string(),
    };
    let fact = delivery_scan_fact(
        rec,
        pending.iter().map(|row| row.event_id.clone()).collect(),
        Some(endpoint_id.to_string()),
        transport.is_live(&endpoint),
        true,
    );
    let effects = state.drive_delivery_scan("pty_send", fact)?;
    for effect in effects {
        if let crate::reconcile::DeliveryEffect::Inject {
            endpoint_id,
            event_ids,
            ..
        } = effect
        {
            let injected =
                inject_planned_messages(state, rec, &transport, &endpoint_id, &event_ids).await?;
            if injected {
                record_message_injection(&rec.pubkey);
            }
            return Ok(injected);
        }
    }
    Ok(false)
}

fn delivery_scan_fact(
    rec: &crate::state::Session,
    pending_event_ids: Vec<String>,
    endpoint_id: Option<String>,
    endpoint_live: bool,
    force: bool,
) -> crate::reconcile::DeliveryScanFact {
    crate::reconcile::DeliveryScanFact {
        pubkey: rec.pubkey.clone(),
        pending_event_ids,
        endpoint_id,
        endpoint_live,
        last_injected_at: last_message_injection(&rec.pubkey),
        debounce_secs: MESSAGE_INJECT_DEBOUNCE_SECS,
        force,
        at: now_secs(),
    }
}
