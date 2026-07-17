//! Transport-owned output presentation, derived separately from message delivery.

/// Whether ordinary session output has no current presentation surface. An ACP
/// session is steerable yet headless; a direct non-PTY harness is headed even
/// though the daemon cannot steer it while idle.
pub(crate) fn session_is_headless(
    store: &crate::state::Store,
    session: &crate::state::Session,
) -> bool {
    let endpoint = match crate::session_host::transport::hosted_endpoint_for(store, session) {
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
    match endpoint {
        crate::session_host::transport::HostedEndpoint::Unhosted => false,
        crate::session_host::transport::HostedEndpoint::Unavailable { .. } => true,
        crate::session_host::transport::HostedEndpoint::Resolved {
            transport,
            endpoint,
        } => !transport.output_is_visible(&endpoint),
    }
}
