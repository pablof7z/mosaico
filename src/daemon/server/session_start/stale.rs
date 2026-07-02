//! Stale-session reaping on harness restart. Extracted from `session_start.rs`
//! to keep the parent under the 500-LOC ceiling (AGENTS.md file-size rule).
//!
//! When a new logical session arrives on the SAME watched pid OR tmux pane (same
//! agent, same work root) it means the harness restarted without emitting a
//! session-end. The previous session's engine task is cancelled, its signer
//! reservation released, and its row marked dead so `who` doesn't show ghosts.
//! (All sessions in this DB are this machine's.)

use super::super::*;
use super::{session_pane, work_root_for_scope};

pub(super) fn cancel_stale_sessions_on_restart(
    state: &Arc<DaemonState>,
    new_session_id: &str,
    agent_slug: &str,
    watch_pid: Option<i32>,
    tmux_pane: Option<&str>,
    new_work_root: &str,
) {
    let alive = state.with_store(|s| s.list_alive_sessions().unwrap_or_default());
    let mut stale_ids: Vec<String> = Vec::new();
    for rec in &alive {
        if rec.session_id == new_session_id || rec.agent_slug != agent_slug {
            continue;
        }
        let same_work_root =
            state.with_store(|s| work_root_for_scope(s, &rec.channel_h)) == new_work_root;
        if !same_work_root {
            continue;
        }
        let same_pid = watch_pid.is_some() && rec.child_pid == watch_pid;
        let same_pane = tmux_pane.is_some_and(|pane| {
            state
                .with_store(|s| session_pane(s, &rec.session_id))
                .as_deref()
                == Some(pane)
        });
        if same_pid || same_pane {
            let reason = if same_pid { "same_pid" } else { "same_pane" };
            tracing::info!(
                stale_session = %rec.session_id,
                new_session = %new_session_id,
                agent = %agent_slug,
                reason,
                "cancelling stale session on harness restart"
            );
            membership_cleanup::remove_session_memberships(
                state,
                &rec.session_id,
                "harness-restart",
            );
            state.release_session_signer(&rec.session_id);
            stale_ids.push(rec.session_id.clone());
        }
    }
    for old_id in stale_ids {
        cancel_session(state, &old_id);
        state.with_store(|s| {
            if let Err(e) = s.mark_dead(&old_id) {
                tracing::error!(
                    stale_session = %old_id,
                    error = %e,
                    "harness-restart reap: failed to mark stale session dead; `who` may show a ghost"
                );
            }
            if let Err(e) = s.mark_identity_dead_for_session(&old_id) {
                tracing::error!(
                    stale_session = %old_id,
                    error = %e,
                    "harness-restart reap: failed to mark stale identity dead"
                );
            }
        });
    }
}
