//! Transport-owned output presentation, derived separately from message delivery.

use crate::session_host::transport::TransportKind;

/// Whether ordinary session output has no current presentation surface. An ACP
/// session is steerable yet headless; a direct non-PTY harness is headed even
/// though the daemon cannot steer it while idle.
pub(crate) fn session_is_headless(
    store: &crate::state::Store,
    session: &crate::state::Session,
) -> bool {
    let Some(kind) = TransportKind::parse(&session.admitted_transport) else {
        return false;
    };
    let locator_kind = match kind {
        TransportKind::Pty => crate::state::LOCATOR_PTY,
        TransportKind::Acp => crate::state::LOCATOR_ACP,
    };
    let endpoint =
        match store.locator_for_session(&session.pubkey, &session.observed_harness, locator_kind) {
            Ok(endpoint) => endpoint,
            Err(e) => {
                tracing::error!(
                    pubkey = %session.pubkey,
                    error = %e,
                    "output-mode check: locator lookup failed; assuming headed"
                );
                return false;
            }
        };
    mode_is_headless(
        kind,
        endpoint.is_some(),
        endpoint.is_some_and(|locator| crate::pty::output_is_visible(&locator.locator_value)),
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
