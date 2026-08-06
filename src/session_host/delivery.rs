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
use crate::session_host::transport::{
    hosted_endpoint_for, transport_for_kind, EndpointRef, HostedEndpoint, TransportKind,
};
pub use doorbell::ring_doorbells;
pub(crate) use output_mode::session_is_headless;
use prompt::inject_planned_messages;

#[cfg(test)]
#[path = "delivery/tests.rs"]
mod delivery_tests;

/// Whether `session` has a live daemon-owned delivery endpoint.
pub(crate) fn session_has_live_delivery_path(
    store: &crate::state::Store,
    session: &crate::state::Session,
) -> bool {
    let endpoint = match hosted_endpoint_for(store, session) {
        Ok(endpoint) => endpoint,
        Err(e) => {
            tracing::error!(
                pubkey = %session.pubkey,
                error = %e,
                "delivery endpoint check: locator lookup failed; assuming unavailable"
            );
            return false;
        }
    };
    match endpoint {
        HostedEndpoint::Resolved {
            transport,
            endpoint,
        } => transport.is_live(&endpoint),
        HostedEndpoint::Unhosted | HostedEndpoint::Unavailable { .. } => false,
    }
}

/// Don't re-inject into the same session within this window (seconds).
/// Coalesces rapid doorbells / double parks; keep short so directed chat stays snappy.
const MESSAGE_INJECT_DEBOUNCE_SECS: u64 = 2;

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
/// the doorbell path. RPC completion is still projected into the same durable
/// lifecycle and native-outcome ledger as an inbox delivery.
pub async fn deliver_spawn_prompt(
    state: &Arc<DaemonState>,
    pubkey: &str,
    endpoint: &EndpointRef,
    text: &str,
) {
    let transport = transport_for_kind(endpoint.kind);
    tokio::time::sleep(transport.opening_delivery_delay()).await;
    let completion = match transport.deliver(endpoint, text, true).await {
        Ok(completion) => completion,
        Err(error) => {
            tracing::warn!(
                endpoint = %endpoint.endpoint_id,
                transport = endpoint.kind.as_str(),
                error = %format!("{error:#}"),
                "failed to deliver spawn prompt"
            );
            return;
        }
    };
    let rec = match state.with_store(|store| store.get_session(pubkey)) {
        Ok(Some(rec)) => rec,
        Ok(None) => {
            tracing::error!(pubkey, "spawn prompt has no registered session");
            return;
        }
        Err(error) => {
            tracing::error!(pubkey, %error, "spawn prompt session lookup failed");
            return;
        }
    };
    if let Err(error) = prompt::track_spawn_prompt(state, &rec, completion).await {
        tracing::error!(pubkey, %error, "failed to track spawn prompt");
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
