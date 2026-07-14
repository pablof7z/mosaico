//! Agent-facing output-mode deltas. The source of truth stays transport-owned;
//! this module only remembers which mode the agent has already been told.

use super::HookContextGraphs;
use crate::state::{Session, Store};
use std::sync::Mutex;

pub(super) fn push_mode_notice(
    graphs: &HookContextGraphs,
    store: &Mutex<Store>,
    rec: &Session,
    announce_initial: bool,
    warnings: &mut Vec<String>,
) {
    let headless = {
        let store = store.lock().expect("store mutex poisoned");
        crate::session_host::session_is_headless(&store, rec)
    };
    let changed = graphs
        .lock()
        .expect("hook-context mutex poisoned")
        .entry(rec.pubkey.clone())
        .or_default()
        .record_headless_mode(headless, announce_initial);
    if changed {
        warnings.push(mode_notice(headless).to_string());
    }
}

fn mode_notice(headless: bool) -> &'static str {
    if headless {
        "Headless mode is on. Your ordinary text output is not currently visible. \
         Publish anything the human or another agent should receive to the relevant channel."
    } else {
        "Headless mode is off. Your ordinary text output is visible in this session."
    }
}

#[cfg(test)]
mod tests {
    use super::mode_notice;

    #[test]
    fn mode_notices_describe_output_without_transport_details() {
        let on = mode_notice(true);
        assert!(on.contains("Headless mode is on."));
        assert!(on.contains("relevant channel"));
        assert!(!on.contains("PTY"));
        assert!(!on.contains("ACP"));
        assert_eq!(
            mode_notice(false),
            "Headless mode is off. Your ordinary text output is visible in this session."
        );
    }
}
