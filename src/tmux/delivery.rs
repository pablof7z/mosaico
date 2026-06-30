use crate::daemon::server::DaemonState;
use crate::tmux::pane::{pane_alive, paste_text, send_enter, tmux_available};
use crate::util::now_secs;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// How long to wait after `session_start` fires before typing into the pane.
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

/// Type the received message into `pane_id` and submit it, so a freshly-spawned
/// harness opens on the message that triggered its spawn.
pub async fn inject_spawn_message(pane_id: &str, text: &str) -> Result<()> {
    tokio::time::sleep(Duration::from_millis(SPAWN_PROMPT_DELAY_MS)).await;
    if pane_alive(pane_id).is_none() {
        anyhow::bail!("pane {pane_id} died before spawn message could be injected");
    }

    paste_text(pane_id, text).await?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    send_enter(pane_id).await?;
    Ok(())
}

struct PendingTmuxPrompt {
    text: String,
    chat_ids: Vec<String>,
}

async fn collect_pending_prompt(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
) -> Result<Option<PendingTmuxPrompt>> {
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
    let rendered = state
        .with_store(|s| crate::injection::render_tmux_mention(s, &chat_rows, &whitelisted, now));
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
        }
        return Ok(None);
    };

    Ok(Some(PendingTmuxPrompt { text, chat_ids }))
}

/// Paste pending mention content into a live pane and submit it as the next
/// prompt. Returns false when no rows were pending. The rows are claimed
/// atomically inside `collect_pending_prompt`; if the paste fails we roll them
/// back to `pending` so a pane that died mid-flight doesn't eat the message.
pub async fn inject_pending_messages_pub(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    pane_id: &str,
) -> Result<bool> {
    let Some(prompt) = collect_pending_prompt(state, rec).await? else {
        return Ok(false);
    };

    if let Err(e) = paste_text(pane_id, &prompt.text).await {
        // Roll the claimed rows back to `pending` so a pane that died mid-flight
        // doesn't eat the message. If the rollback fails the rows stay
        // `delivered` and are never retried — that defeats the guarantee, so log
        // it loudly instead of dropping it with `.ok()`.
        if let Err(re) = state.with_store(|s| s.reenqueue_pending(&prompt.chat_ids, &rec.session_id))
        {
            tracing::error!(
                session_id = %rec.session_id,
                error = %re,
                "failed to roll back claimed inbox rows after paste failure; mention may be lost"
            );
        }
        return Err(e);
    }
    // Record exactly what we pasted so the resulting user-prompt-submit hook is
    // recognized as an echo and not republished into the channel.
    state.record_injection_echo(&rec.session_id, &prompt.text);
    tokio::time::sleep(Duration::from_millis(200)).await;
    send_enter(pane_id).await?;
    Ok(true)
}

/// Scans for sessions with unread inbox rows that have a live tmux endpoint,
/// and have not been injected recently.
pub fn ring_doorbells(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        if let Err(e) = ring_doorbells_inner(&state).await {
            tracing::error!(error = %format!("{e:#}"), "ring_doorbells: doorbell scan failed");
        }
    });
}

async fn ring_doorbells_inner(state: &Arc<DaemonState>) -> Result<()> {
    if !tmux_available() {
        return Ok(());
    }

    let sessions_with_chat: Vec<crate::state::Session> = state.with_store(|s| {
        let alive = match s.list_alive_sessions() {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e, "ring_doorbells: list_alive_sessions failed — skipping doorbell scan this tick");
                return Vec::new();
            }
        };
        alive
            .into_iter()
            .filter(|rec| {
                !rec.working
                    && match s.drain_pending_for_session(&rec.session_id) {
                        Ok(pending) => !pending.is_empty(),
                        Err(e) => {
                            tracing::error!(session_id = %rec.session_id, error = %e, "ring_doorbells: drain_pending_for_session failed — treating session as having no pending inbox");
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

        // The tmux pane is the session's `tmux_pane` alias (reused panes repoint to
        // the newest owner), so the alias IS the endpoint.
        let pane_id = match state.with_store(|s| match s.aliases_for_session(&sid) {
            Ok(aliases) => aliases
                .into_iter()
                .find(|a| a.external_id_kind == "tmux_pane")
                .map(|a| a.external_id),
            Err(e) => {
                tracing::error!(session_id = %sid, error = %e, "ring_doorbells: aliases_for_session failed — cannot resolve pane endpoint this tick");
                None
            }
        }) {
            Some(p) => p,
            None => continue,
        };

        if pane_alive(&pane_id).is_none() {
            if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                eprintln!("[tmux] pane {pane_id} gone; removing endpoint for {sid}");
            }
            if let Err(e) = state.with_store(|s| s.clear_tmux_pane(&sid)) {
                tracing::error!(session_id = %sid, error = %e, "ring_doorbells: clear_tmux_pane failed — stale dead-pane endpoint retained");
            }
            continue;
        }

        record_message_injection(&sid);

        match inject_pending_messages_pub(state, &rec, &pane_id).await {
            Ok(true) => {
                if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                    eprintln!(
                        "[tmux] pending messages injected into pane {pane_id} for session {sid}"
                    );
                }
            }
            Ok(false) => {}
            Err(e) => {
                if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                    eprintln!(
                        "[tmux] pending message inject failed for {sid} pane {pane_id}: {e:#}"
                    );
                }
            }
        }
    }
    Ok(())
}
