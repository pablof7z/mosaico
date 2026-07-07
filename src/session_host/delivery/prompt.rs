use crate::daemon::server::DaemonState;
use crate::util::now_secs;
use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;

struct PendingPrompt {
    text: String,
    chat_ids: Vec<String>,
}

async fn collect_pending_prompt(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    event_ids: &[String],
) -> Result<Option<PendingPrompt>> {
    let now = now_secs();
    let mut chat_rows = state
        .with_store(|s| s.claim_pending_event_ids_for_session(event_ids, &rec.session_id, now))?;
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

pub(super) async fn inject_planned_messages_pty(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    pty_id: &str,
    event_ids: &[String],
) -> Result<bool> {
    if !crate::pty::is_live(pty_id) {
        anyhow::bail!("pty session {pty_id} is not live");
    }
    let Some(prompt) = collect_pending_prompt(state, rec, event_ids).await? else {
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
