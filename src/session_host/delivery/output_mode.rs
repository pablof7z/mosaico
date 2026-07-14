//! Transport-owned output presentation, derived separately from message delivery.

use crate::session_host::transport::{transport_kind_for_slug, TransportKind};

/// Whether ordinary session output has no current presentation surface. An ACP
/// session is steerable yet headless; a direct non-PTY harness is headed even
/// though the daemon cannot steer it while idle.
pub(crate) fn session_is_headless(
    store: &crate::state::Store,
    session: &crate::state::Session,
) -> bool {
    let kind = transport_kind_for_slug(&session.agent_slug);
    let locator_kind = match kind {
        TransportKind::Pty => crate::state::LOCATOR_PTY,
        TransportKind::Acp => crate::state::LOCATOR_ACP,
    };
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
    let endpoint = locators
        .iter()
        .find(|locator| locator.locator_kind == locator_kind)
        .map(|locator| locator.locator_value.as_str());
    let pty_output_visible =
        matches!(kind, TransportKind::Pty) && endpoint.is_some_and(crate::pty::output_is_visible);
    mode_is_headless(kind, endpoint.is_some(), pty_output_visible)
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
