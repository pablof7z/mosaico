use crate::daemon::server::DaemonState;
use crate::util::now_secs;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

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

fn is_debounced(session_id: &str) -> bool {
    let m = debounce().lock().unwrap();
    m.get(session_id)
        .map(|&t| now_secs().saturating_sub(t) < MESSAGE_INJECT_DEBOUNCE_SECS)
        .unwrap_or(false)
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

struct PendingPrompt {
    text: String,
    chat_ids: Vec<String>,
}

async fn collect_pending_prompt(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
) -> Result<Option<PendingPrompt>> {
    let now = now_secs();
    // Atomic claim: this transitions the rows to `delivered` and returns them in
    // one statement, so a racing hook can't also deliver them (atomicity IS the
    // dedup). If the paste later fails, the caller re-enqueues them.
    let mut chat_rows = state.with_store(|s| s.claim_pending_for_session(&rec.session_id, now))?;
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
        // Defensive: nothing to paste though rows were claimed — give them back.
        // If the rollback itself fails the rows stay `delivered` and the mention
        // is silently lost, so surface that loudly rather than swallow it.
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

    Ok(Some(PendingPrompt { text, chat_ids }))
}

pub async fn inject_pending_messages_pty(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    pty_id: &str,
) -> Result<bool> {
    let Some(prompt) = collect_pending_prompt(state, rec).await? else {
        return Ok(false);
    };

    if let Err(e) = crate::pty::inject(pty_id, &prompt.text, true, false) {
        if let Err(re) =
            state.with_store(|s| s.reenqueue_pending(&prompt.chat_ids, &rec.session_id))
        {
            tracing::error!(
                session_id = %rec.session_id,
                error = %re,
                "failed to roll back claimed inbox rows after pty inject failure; mention may be lost"
            );
            state.emit_delivery_failure(
                &rec.channel_h,
                &rec.agent_slug,
                &rec.session_id,
                format!("failed to roll back claimed inbox rows after pty inject failure: {re:#}"),
            );
        }
        return Err(e);
    }
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
    tokio::time::sleep(Duration::from_millis(200)).await;
    crate::pty::inject(pty_id, "", false, true)?;
    Ok(true)
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
    let sessions_with_chat: Vec<crate::state::Session> = state.with_store(|s| {
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
            .into_iter()
            .filter(|rec| {
                !rec.working
                    && match s.peek_pending_for_session(&rec.session_id) {
                        Ok(pending) => !pending.is_empty(),
                        Err(e) => {
                            tracing::error!(session_id = %rec.session_id, error = %e, "ring_doorbells: peek_pending_for_session failed — treating session as having no pending inbox");
                            state.emit_delivery_failure(
                                &rec.channel_h,
                                &rec.agent_slug,
                                &rec.session_id,
                                format!(
                                    "failed to read pending inbox for doorbell scan: {e:#}"
                                ),
                            );
                            false
                        }
                    }
            })
            .collect()
    });

    for rec in sessions_with_chat {
        let sid = rec.session_id.clone();
        if is_debounced(&sid) {
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

        if let Some(pty_id) = aliases
            .iter()
            .find(|a| a.external_id_kind == "pty_session")
            .map(|a| a.external_id.clone())
        {
            if crate::pty::is_live(&pty_id) {
                record_message_injection(&sid);
                match inject_pending_messages_pty(state, &rec, &pty_id).await {
                    Ok(true) => {
                        if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                            eprintln!(
                                "[pty] pending messages injected into session {pty_id} for {sid}"
                            );
                        }
                    }
                    Ok(false) => {}
                    Err(e) => {
                        state.emit_delivery_failure(
                            &rec.channel_h,
                            &rec.agent_slug,
                            &sid,
                            format!("pending message injection failed for pty {pty_id}: {e:#}"),
                        );
                    }
                }
                continue;
            }
            let _ = state.with_store(|s| s.clear_alias_kind(&sid, "pty_session"));
            let _ = state.with_store(|s| s.clear_alias_kind(&sid, "pty_socket"));
        }
    }
    Ok(())
}
