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
    transport_kind_for_slug, AcpTransport, EndpointRef, SessionTransport, TransportKind,
};
pub use doorbell::ring_doorbells;
#[cfg(test)]
pub(crate) use output_mode::headless_for_endpoint;
pub(crate) use output_mode::session_is_headless;
use prompt::{inject_planned_messages_acp, inject_planned_messages_pty};

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
    kind: TransportKind,
) -> Result<Option<String>> {
    Ok(store
        .locators_for_pubkey(pubkey)?
        .into_iter()
        .find(|locator| locator.locator_kind == locator_kind(kind))
        .map(|locator| locator.locator_value))
}

/// Liveness of a session's typed transport endpoint.
fn endpoint_is_live(kind: TransportKind, endpoint_id: &str) -> bool {
    match kind {
        TransportKind::Pty => crate::pty::is_live(endpoint_id),
        TransportKind::Acp => AcpTransport.is_live(&EndpointRef {
            kind: TransportKind::Acp,
            endpoint_id: endpoint_id.to_string(),
        }),
    }
}

/// Whether `session` has a live daemon-owned delivery endpoint.
pub(crate) fn session_has_live_delivery_path(
    store: &crate::state::Store,
    session: &crate::state::Session,
) -> bool {
    let locators = match store.locators_for_pubkey(&session.pubkey) {
        Ok(locators) => locators,
        Err(e) => {
            tracing::error!(
                pubkey = %session.pubkey,
                error = %e,
                "delivery endpoint check: locator lookup failed; assuming unavailable"
            );
            return false;
        }
    };
    locators
        .into_iter()
        .any(|locator| match locator.locator_kind.as_str() {
            crate::state::LOCATOR_PTY => {
                endpoint_is_live(TransportKind::Pty, &locator.locator_value)
            }
            crate::state::LOCATOR_ACP => {
                endpoint_is_live(TransportKind::Acp, &locator.locator_value)
            }
            _ => false,
        })
}

/// How long to wait after `session_start` fires before typing into the PTY.
/// The hook fires early in harness startup; we need to wait until the input
/// box is actually interactive.
const SPAWN_PROMPT_DELAY_MS: u64 = 2000;

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

/// Type the received message into `endpoint_id` and submit it, so a freshly-spawned
/// harness opens on the message that triggered its spawn.
pub async fn inject_spawn_message(endpoint_id: &str, text: &str) -> Result<()> {
    tokio::time::sleep(Duration::from_millis(SPAWN_PROMPT_DELAY_MS)).await;
    if !crate::pty::is_live(endpoint_id) {
        anyhow::bail!("pty session {endpoint_id} died before spawn message could be injected");
    }

    crate::pty::inject(endpoint_id, text, true, true)?;
    Ok(())
}

/// Deliver a fresh session's opening prompt over whichever transport hosts it:
/// ACP via a JSON-RPC `deliver` (submit=true → a fresh turn), PTY via the
/// bracketed-paste spawn inject. The ACP child lives in the daemon registry, so
/// this MUST run in the daemon (the caller that spawned it). Failures are logged,
/// not propagated: the session is already live and can still receive mentions via
/// the doorbell path.
pub async fn deliver_spawn_prompt(agent_slug: &str, endpoint_id: &str, text: &str) {
    match transport_kind_for_slug(agent_slug) {
        Ok(TransportKind::Acp) => {
            let ep = EndpointRef {
                kind: TransportKind::Acp,
                endpoint_id: endpoint_id.to_string(),
            };
            if let Err(e) = AcpTransport.deliver(&ep, text, true).await {
                tracing::warn!(
                    agent = %agent_slug,
                    endpoint = %endpoint_id,
                    error = %format!("{e:#}"),
                    "failed to deliver ACP spawn prompt"
                );
            }
        }
        Ok(TransportKind::Pty) => {
            if let Err(e) = inject_spawn_message(endpoint_id, text).await {
                tracing::warn!(
                    agent = %agent_slug,
                    endpoint = %endpoint_id,
                    error = %format!("{e:#}"),
                    "failed to inject PTY spawn prompt"
                );
            }
        }
        Err(e) => tracing::warn!(
            agent = %agent_slug,
            error = %format!("{e:#}"),
            "failed to resolve transport for spawn prompt"
        ),
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
    let fact = delivery_scan_fact(
        rec,
        pending.iter().map(|row| row.event_id.clone()).collect(),
        Some(endpoint_id.to_string()),
        crate::pty::is_live(endpoint_id),
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
                inject_planned_messages_pty(state, rec, &endpoint_id, &event_ids).await?;
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
