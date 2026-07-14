//! Transport-owned output presentation, derived separately from message delivery.

use crate::session_host::transport::{transport_kind_for_slug, TransportKind};

/// Whether ordinary session output has no current presentation surface. An ACP
/// session is steerable yet headless; a direct non-PTY harness is headed even
/// though the daemon cannot steer it while idle.
pub(crate) fn session_is_headless(
    store: &crate::state::Store,
    session: &crate::state::Session,
) -> bool {
    let aliases = match store.aliases_for_session(&session.session_id) {
        Ok(aliases) => aliases,
        Err(e) => {
            tracing::error!(
                session = %session.session_id,
                error = %e,
                "output-mode check: aliases lookup failed; assuming headed"
            );
            return false;
        }
    };
    let endpoint = aliases
        .iter()
        .find(|a| a.external_id_kind == "pty_session")
        .map(|a| a.external_id.as_str());
    let pty_output_visible = endpoint.is_some_and(crate::pty::output_is_visible);
    mode_is_headless(
        transport_kind_for_slug(&session.agent_slug),
        endpoint.is_some(),
        pty_output_visible,
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
