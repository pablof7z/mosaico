//! Agent-facing output-mode deltas. The source of truth stays transport-owned;
//! this module only remembers which mode the agent has already been told.

use super::HookContextStates;
use crate::state::Session;

pub(super) fn push_mode_notice(
    store: &std::sync::Mutex<crate::state::Store>,
    states: &HookContextStates,
    rec: &Session,
    announce_initial: bool,
    warnings: &mut Vec<String>,
) {
    let headless = {
        let store = store.lock().expect("store mutex poisoned");
        crate::session_host::session_is_headless(&store, rec)
    };
    let changed = states
        .lock()
        .expect("hook-context mutex poisoned")
        .entry(rec.pubkey.clone())
        .or_default()
        .record_headless_mode(headless, announce_initial);
    if changed && headless {
        warnings.push(mode_notice().to_string());
    }
}

fn mode_notice() -> &'static str {
    "Headless mode is on. Your ordinary text output is not currently visible. \
     Publish anything the human or another agent should receive to the relevant channel."
}

#[cfg(test)]
mod tests {
    use super::mode_notice;

    #[test]
    fn headless_notice_describes_output_without_transport_details() {
        let on = mode_notice();
        assert!(on.contains("Headless mode is on."));
        assert!(on.contains("relevant channel"));
        assert!(!on.contains("PTY"));
        assert!(!on.contains("ACP"));
    }
}
