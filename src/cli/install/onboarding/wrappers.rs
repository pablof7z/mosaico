//! The shell-wrapper decision made alongside harness selection.
//!
//! A wrapper is an alias that sends a native harness command through Mosaico.
//! Offering one only makes sense when this machine has a profile Mosaico can
//! own, so the whole choice is hidden when it does not.

use super::super::config::Harness;
use super::super::shell;

/// Whether wrappers can be offered at all, and which are already installed.
pub(super) fn initial_state(all: &[Harness]) -> (bool, Vec<bool>) {
    let configured = shell::configured_wrappers(all).unwrap_or_default();
    (
        shell::supported(),
        all.iter().map(|h| configured.contains(h.id)).collect(),
    )
}

/// Toggle the wrapper under the cursor. Wrapping a command implies installing
/// that harness, so turning it on selects the harness rather than promising an
/// alias to something Mosaico never wired.
pub(super) fn toggle(selected: &mut [bool], wrapped: &mut [bool], cursor: usize) {
    let Some(flag) = wrapped.get_mut(cursor) else {
        return;
    };
    *flag = !*flag;
    if *flag {
        if let Some(on) = selected.get_mut(cursor) {
            *on = true;
        }
    }
}

/// Harnesses that end up with a Mosaico-owned alias: wrapped and installed.
pub(super) fn ids(all: &[Harness], selected: &[bool], wrapped: &[bool]) -> Vec<&'static str> {
    all.iter()
        .enumerate()
        .filter(|(i, _)| on(wrapped, *i) && on(selected, *i))
        .map(|(_, h)| h.id)
        .collect()
}

fn on(flags: &[bool], index: usize) -> bool {
    flags.get(index).copied().unwrap_or(false)
}
