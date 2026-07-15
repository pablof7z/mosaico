//! Transport-owned output presentation, derived separately from message delivery.

use crate::session_host::transport::TransportKind;

/// Whether ordinary session output has no current presentation surface. An ACP
/// session is steerable yet headless; a direct non-PTY harness is headed even
/// though the daemon cannot steer it while idle.
pub(crate) fn session_is_headless(
    store: &crate::state::Store,
    session: &crate::state::Session,
) -> bool {
    let locators = match store.locators_for_pubkey(&session.pubkey) {
        Ok(locators) => locators,
        Err(e) => {
            tracing::error!(
                pubkey = %session.pubkey,
                error = %e,
                "output-mode check: locator lookup failed; assuming headed"
            );
            return false;
        }
    };
    if locators
        .iter()
        .any(|locator| locator.locator_kind == crate::state::LOCATOR_ACP)
    {
        return true;
    }
    let pty = locators
        .iter()
        .find(|locator| locator.locator_kind == crate::state::LOCATOR_PTY);
    mode_is_headless(
        TransportKind::Pty,
        pty.is_some(),
        pty.is_some_and(|locator| crate::pty::output_is_visible(&locator.locator_value)),
    )
}

#[cfg(test)]
pub(crate) fn headless_for_endpoint(
    kind: TransportKind,
    has_endpoint: bool,
    pty_output_visible: bool,
) -> bool {
    mode_is_headless(kind, has_endpoint, pty_output_visible)
}

fn mode_is_headless(kind: TransportKind, has_endpoint: bool, pty_output_visible: bool) -> bool {
    match kind {
        TransportKind::Acp => true,
        TransportKind::Pty => has_endpoint && !pty_output_visible,
    }
}
