use crate::daemon::server::DaemonState;
use crate::util::now_secs;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

mod prompt;
use prompt::inject_planned_messages_pty;

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

fn last_message_injection(session_id: &str) -> Option<u64> {
    debounce().lock().unwrap().get(session_id).copied()
}

fn record_message_injection(session_id: &str) {
    debounce()
        .lock()
        .unwrap()
        .insert(session_id.to_string(), now_secs());
}

fn prune_debounce(active_session_ids: &HashSet<String>) {
    let now = now_secs();
    debounce().lock().unwrap().retain(|session_id, last| {
        active_session_ids.contains(session_id)
            && now.saturating_sub(*last) < MESSAGE_INJECT_DEBOUNCE_SECS
    });
}

/// Type the received message into `pty_id` and submit it, so a freshly-spawned
/// harness opens on the message that triggered its spawn.
pub async fn inject_spawn_message(pty_id: &str, text: &str) -> Result<()> {
    tokio::time::sleep(Duration::from_millis(SPAWN_PROMPT_DELAY_MS)).await;
    if !crate::pty::is_live(pty_id) {
        anyhow::bail!("pty session {pty_id} died before spawn message could be injected");
    }

    crate::pty::inject(pty_id, text, true, true)?;
    Ok(())
}

pub async fn inject_pending_messages_pty(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    pty_id: &str,
) -> Result<bool> {
    let pending = state.with_store(|s| s.peek_pending_for_session(&rec.session_id))?;
    if pending.is_empty() {
        return Ok(false);
    };
    let fact = delivery_scan_fact(
        rec,
        pending.iter().map(|row| row.event_id.clone()).collect(),
        Some(pty_id.to_string()),
        crate::pty::is_live(pty_id),
        true,
    );
    let effects = state.drive_delivery_scan("pty_send", fact)?;
    for effect in effects {
        if let crate::reconcile::DeliveryEffect::Inject {
            pty_id, event_ids, ..
        } = effect
        {
            let injected = inject_planned_messages_pty(state, rec, &pty_id, &event_ids).await?;
            if injected {
                record_message_injection(&rec.session_id);
            }
            return Ok(injected);
        }
    }
    Ok(false)
}

fn delivery_scan_fact(
    rec: &crate::state::Session,
    pending_event_ids: Vec<String>,
    pty_id: Option<String>,
    pty_live: bool,
    force: bool,
) -> crate::reconcile::DeliveryScanFact {
    crate::reconcile::DeliveryScanFact {
        session_id: rec.session_id.clone(),
        pending_event_ids,
        working: rec.working,
        pty_id,
        pty_live,
        last_injected_at: last_message_injection(&rec.session_id),
        debounce_secs: MESSAGE_INJECT_DEBOUNCE_SECS,
        force,
        at: now_secs(),
    }
}

/// Scans for sessions with unread inbox rows that have a live PTY endpoint,
/// and have not been injected recently.
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
        let active_ids: HashSet<String> = alive.iter().map(|rec| rec.session_id.clone()).collect();
        prune_debounce(&active_ids);
        alive
    });

    for rec in sessions {
        let sid = rec.session_id.clone();
        let pending = match state.with_store(|s| s.peek_pending_for_session(&sid)) {
            Ok(pending) => pending,
            Err(e) => {
                tracing::error!(session_id = %sid, error = %e, "ring_doorbells: peek_pending_for_session failed");
                state.emit_delivery_failure(
                    &rec.channel_h,
                    &rec.agent_slug,
                    &sid,
                    format!("failed to read pending inbox for doorbell scan: {e:#}"),
                );
                continue;
            }
        };
        if pending.is_empty() {
            continue;
        }

        let aliases = match state.with_store(|s| s.aliases_for_session(&sid)) {
            Ok(aliases) => aliases,
            Err(e) => {
                tracing::error!(session_id = %sid, error = %e, "ring_doorbells: aliases_for_session failed — cannot resolve endpoint this tick");
                state.emit_delivery_failure(
                    &rec.channel_h,
                    &rec.agent_slug,
                    &sid,
                    format!("failed to resolve delivery endpoint: {e:#}"),
                );
                continue;
            }
        };

        let pty_id = aliases
            .iter()
            .find(|a| a.external_id_kind == "pty_session")
            .map(|a| a.external_id.clone());
        let pty_live = pty_id.as_deref().is_some_and(crate::pty::is_live);
        let fact = delivery_scan_fact(
            &rec,
            pending.into_iter().map(|row| row.event_id).collect(),
            pty_id,
            pty_live,
            false,
        );
        let effects = match state.drive_delivery_scan("doorbell", fact) {
            Ok(effects) => effects,
            Err(e) => {
                state.emit_delivery_failure(
                    &rec.channel_h,
                    &rec.agent_slug,
                    &sid,
                    format!("delivery reconciler failed: {e:#}"),
                );
                continue;
            }
        };
        apply_delivery_effects(state, &rec, effects).await;
    }
    Ok(())
}

async fn apply_delivery_effects(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    effects: Vec<crate::reconcile::DeliveryEffect>,
) {
    for effect in effects {
        match effect {
            crate::reconcile::DeliveryEffect::Inject {
                session_id,
                pty_id,
                event_ids,
            } => match inject_planned_messages_pty(state, rec, &pty_id, &event_ids).await {
                Ok(true) => {
                    record_message_injection(&session_id);
                    if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                        eprintln!(
                                "[pty] pending messages injected into session {pty_id} for {session_id}"
                            );
                    }
                }
                Ok(false) => {}
                Err(e) => state.emit_delivery_failure(
                    &rec.channel_h,
                    &rec.agent_slug,
                    &session_id,
                    format!("pending message injection failed for pty {pty_id}: {e:#}"),
                ),
            },
            crate::reconcile::DeliveryEffect::RetryAfter {
                session_id,
                delay_secs,
            } => schedule_delivery_retry(state.clone(), session_id, delay_secs),
            crate::reconcile::DeliveryEffect::ClearDeadEndpoint { session_id } => {
                let _ = state.with_store(|s| s.clear_alias_kind(&session_id, "pty_session"));
                let _ = state.with_store(|s| s.clear_alias_kind(&session_id, "pty_socket"));
            }
        }
    }
}

fn schedule_delivery_retry(state: Arc<DaemonState>, session_id: String, delay_secs: u64) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(delay_secs.max(1))).await;
        if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
            eprintln!("[pty] retrying deferred delivery for {session_id}");
        }
        ring_doorbells(state);
    });
}
