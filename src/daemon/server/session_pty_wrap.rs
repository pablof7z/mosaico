//! `session_pty_wrap` — re-home the CALLER's OWN session into a fresh
//! daemon-owned PTY supervisor.
//!
//! An agent whose harness was started manually outside a daemon-owned PTY
//! (e.g. `codex --yolo resume <id>` typed directly into an iTerm tab) has no
//! `pty_session` alias. Nothing can inject a mention into it, so idle
//! mentions silently black-hole. This RPC lets that session re-home ITSELF:
//! kill the manually-started process and resume the SAME harness session
//! (same resume token, same channel) inside a fresh daemon PTY supervisor,
//! which registers the standard `pty_session` alias.
//!
//! The whole operation (refusal checks, killing the old process, resuming
//! into a fresh PTY) runs server-side in one RPC so it cannot race a second
//! CLI round-trip claiming the same session — see `AGENTS.md` on avoiding
//! double-spawn/claim races.
//!
//! Caveat (documented, not fixed here): resume replays only what the harness
//! itself persisted (its own transcript/session file); terminal scrollback
//! from the killed process is lost. This is the same limitation as every
//! other resume path in this codebase (`pty_resume`, offline-mention resume).

use super::pty_rpc::resume_token_for;
use super::session_end::rpc_session_kill;
use super::*;

#[derive(serde::Deserialize)]
struct SessionPtyWrapParams {
    session: String,
}

/// How a re-home request resolves, as a pure function of the session's
/// current state. Kept separate from the RPC handler so the branch logic is
/// unit-testable without a live daemon, store, or PTY.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::server) enum PtyWrapDecision {
    /// Kill the old process and resume into a fresh daemon PTY.
    Wrap,
    /// A live `pty_session` alias already exists — nothing to do.
    AlreadyWrapped,
    /// The session is mid-turn (`working=1`) — refuse to interrupt it.
    Working,
    /// The session carries no harness-native resume token, so it cannot be
    /// replayed into a fresh process.
    NotResumable,
}

pub(in crate::daemon::server) fn decide_pty_wrap(
    working: bool,
    already_wrapped_live: bool,
    resumable: bool,
) -> PtyWrapDecision {
    if already_wrapped_live {
        PtyWrapDecision::AlreadyWrapped
    } else if working {
        PtyWrapDecision::Working
    } else if !resumable {
        PtyWrapDecision::NotResumable
    } else {
        PtyWrapDecision::Wrap
    }
}

fn refusal(refusal: &str, reason: impl Into<String>) -> serde_json::Value {
    serde_json::json!({ "wrapped": false, "refusal": refusal, "reason": reason.into() })
}

/// The `pty_session` alias for a session, if it currently resolves to a LIVE
/// endpoint. Mirrors the doorbell scan / `session_end`'s PTY-endpoint lookup
/// (`src/session_host/delivery.rs`, `src/daemon/server/session_end.rs`).
fn live_pty_alias(state: &Arc<DaemonState>, session_id: &str) -> Option<String> {
    let pty_id = state
        .with_store(|s| s.aliases_for_session(session_id))
        .ok()?
        .into_iter()
        .find(|a| a.external_id_kind == "pty_session")
        .map(|a| a.external_id)?;
    crate::pty::is_live(&pty_id).then_some(pty_id)
}

pub(in crate::daemon::server) async fn rpc_session_pty_wrap(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: SessionPtyWrapParams =
        serde_json::from_value(params.clone()).context("parsing session_pty_wrap params")?;
    let Some(rec) = state.with_store(|s| s.get_session(&p.session).ok().flatten()) else {
        return Ok(refusal("not_found", "no local session matched"));
    };

    let already_wrapped = live_pty_alias(state, &rec.session_id).is_some();
    let resume_id = resume_token_for(&rec);
    let decision = decide_pty_wrap(rec.working, already_wrapped, resume_id.is_some());

    let resume_id = match decision {
        PtyWrapDecision::AlreadyWrapped => {
            return Ok(refusal(
                "already_wrapped",
                "session is already inside a live daemon PTY; nothing to do",
            ));
        }
        PtyWrapDecision::Working => {
            return Ok(refusal(
                "working",
                "session is mid-turn (working); refusing to interrupt in-flight work",
            ));
        }
        PtyWrapDecision::NotResumable => {
            return Ok(refusal(
                "not_resumable",
                "session has no harness resume token; cannot re-home into a fresh PTY",
            ));
        }
        PtyWrapDecision::Wrap => {
            resume_id.expect("Wrap implies decide_pty_wrap saw a resume token")
        }
    };

    let slug = rec.agent_slug.clone();
    let scope = rec.channel_h.clone();
    let session_id = rec.session_id.clone();

    // Kill the old (non-PTY) process and mark the session dead BEFORE the
    // resumed session registers. Ordering matters: resuming first would let
    // the fresh PTY's session-start race the old row's still-alive claim on
    // the same (pubkey, channel), risking a double-inject.
    let kill = rpc_session_kill(state, &serde_json::json!({ "session": session_id })).await?;
    if !kill["killed"].as_bool().unwrap_or(false) {
        let reason = kill["reason"].as_str().unwrap_or("unknown");
        return Ok(refusal(
            "kill_failed",
            format!("could not stop the existing process before re-homing: {reason}"),
        ));
    }

    match crate::session_host::resume_agent(state, &slug, &scope, &resume_id).await {
        Ok(pty_id) => Ok(serde_json::json!({
            "wrapped": true,
            "pty_id": pty_id,
            "session_id": session_id,
        })),
        Err(e) => Ok(refusal(
            "resume_failed",
            format!("old session was ended, but re-homing into a fresh PTY failed: {e:#}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_when_idle_unwrapped_and_resumable() {
        assert_eq!(decide_pty_wrap(false, false, true), PtyWrapDecision::Wrap);
    }

    #[test]
    fn refuses_already_wrapped_regardless_of_working() {
        assert_eq!(
            decide_pty_wrap(false, true, true),
            PtyWrapDecision::AlreadyWrapped
        );
        assert_eq!(
            decide_pty_wrap(true, true, true),
            PtyWrapDecision::AlreadyWrapped
        );
    }

    #[test]
    fn refuses_mid_turn_session() {
        assert_eq!(decide_pty_wrap(true, false, true), PtyWrapDecision::Working);
    }

    #[test]
    fn refuses_when_no_resume_token() {
        assert_eq!(
            decide_pty_wrap(false, false, false),
            PtyWrapDecision::NotResumable
        );
    }
}
