//! Agent-facing heads-up for a stalled status/title distiller.
//!
//! A run of failed background status/title updates is otherwise invisible to the
//! agent (it only surfaces in a per-session debug log). This surfaces it as a
//! turn-start warning — throttled, so a persistent outage nags the agent a few
//! times an hour rather than on every turn.

use std::sync::Mutex;

use crate::state::{Session, Store};

/// Consecutive failures before the agent-facing heads-up fires — absorbs a
/// single transient blip without alarming the agent.
const DISTILL_NOTICE_MIN_STREAK: u64 = 3;
/// Minimum gap between repeats of the heads-up while the failure streak
/// persists: "a few times per hour", not every turn.
const DISTILL_NOTICE_THROTTLE_SECS: u64 = 15 * 60;

/// Push the stalled-distiller heads-up onto `warnings` when the failure streak
/// has crossed the threshold and the throttle window has elapsed, stamping the
/// notice time so it does not repeat next turn.
pub(super) fn push_heads_up(
    store: &Mutex<Store>,
    rec: &Session,
    now: u64,
    warnings: &mut Vec<String>,
) {
    if rec.distill_fail_streak < DISTILL_NOTICE_MIN_STREAK
        || now.saturating_sub(rec.distill_notice_at) < DISTILL_NOTICE_THROTTLE_SECS
    {
        return;
    }
    warnings.push(
        "This session's status/title updates haven't been generating \
         successfully for a while. You may want to let the user know \
         status updates aren't working right now."
            .to_string(),
    );
    let s = store.lock().expect("store mutex poisoned");
    if let Err(e) = s.mark_distill_notice(&rec.session_id, now) {
        tracing::error!(
            session = %rec.session_id,
            error = ?e,
            "turn_start: mark_distill_notice failed; heads-up may repeat next turn"
        );
    }
}
