//! Single process-termination authority for admitted session generations.

use super::*;
use crate::session_host::transport::{HostedEndpoint, TransportKind};
use crate::state::{RuntimeState, Session};

mod shutdown;

#[derive(Debug)]
pub(super) enum AutomaticTerminationOutcome {
    Terminated { locator_kind: Option<&'static str> },
    PresentationChanged(crate::pty::PresentationSnapshot),
}

/// Automatically terminate only when the transport can prove the runtime is
/// not user-attached. Loss of the control channel is uncertainty, never
/// authorization to fall back to raw process signals.
pub(super) async fn terminate_automatic_if_unattached(
    state: &Arc<DaemonState>,
    session: &Session,
) -> Result<AutomaticTerminationOutcome> {
    let endpoint = state
        .with_store(|store| crate::session_host::transport::hosted_endpoint_for(store, session))?;
    match endpoint {
        HostedEndpoint::Resolved {
            endpoint,
            transport: _,
        } if endpoint.kind == TransportKind::Pty => {
            match crate::pty::kill_if_headless_at(&endpoint.endpoint_id, session.attachment_epoch) {
                Ok(crate::pty::ConditionalKillOutcome::Killed { .. }) => {
                    Ok(AutomaticTerminationOutcome::Terminated {
                        locator_kind: Some(endpoint.kind.locator_kind()),
                    })
                }
                Ok(crate::pty::ConditionalKillOutcome::PresentationChanged { presentation }) => Ok(
                    AutomaticTerminationOutcome::PresentationChanged(presentation),
                ),
                Err(error) => {
                    record_control_unavailable(state, session, now_secs())?;
                    Err(error.into())
                }
            }
        }
        HostedEndpoint::Resolved {
            transport,
            endpoint,
        } => {
            transport.kill(&endpoint).await?;
            wait_for_process_exit(|| !transport.is_live(&endpoint)).await?;
            Ok(AutomaticTerminationOutcome::Terminated {
                locator_kind: Some(endpoint.kind.locator_kind()),
            })
        }
        HostedEndpoint::Unavailable { kind } => {
            record_control_unavailable(state, session, now_secs())?;
            anyhow::bail!(
                "refusing automatic termination of {} runtime without its owned endpoint",
                kind.as_str()
            );
        }
        HostedEndpoint::Unhosted => {
            if tracked_process_alive(session) {
                record_control_unavailable(state, session, now_secs())?;
                anyhow::bail!(
                    "refusing automatic termination of unbound live runtime generation {}",
                    session.runtime_generation
                );
            }
            Ok(AutomaticTerminationOutcome::Terminated { locator_kind: None })
        }
    }
}

/// Explicit operator/revocation termination may stop an attached runtime, but
/// still requires exact endpoint ownership and confirmed process exit.
pub(super) async fn terminate_explicit(
    state: &Arc<DaemonState>,
    session: &Session,
) -> Result<String> {
    if session.runtime_state == RuntimeState::Stopped {
        return Ok("runtime already stopped".into());
    }
    match state
        .with_store(|store| crate::session_host::transport::hosted_endpoint_for(store, session))?
    {
        HostedEndpoint::Resolved {
            transport,
            endpoint,
        } if endpoint.kind == TransportKind::Pty => {
            terminate_explicit_pty(&transport, &endpoint, session).await?;
            clear_runtime_locator(state, session, endpoint.kind.locator_kind())?;
            Ok(format!("endpoint={}", endpoint.endpoint_id))
        }
        HostedEndpoint::Resolved {
            transport,
            endpoint,
        } => {
            if transport.is_live(&endpoint) {
                transport
                    .kill(&endpoint)
                    .await
                    .with_context(|| format!("killing {} endpoint", endpoint.kind.as_str()))?;
                wait_for_process_exit(|| !transport.is_live(&endpoint)).await?;
            }
            clear_runtime_locator(state, session, endpoint.kind.locator_kind())?;
            Ok(format!("endpoint={}", endpoint.endpoint_id))
        }
        HostedEndpoint::Unavailable { kind } => {
            anyhow::bail!(
                "session {} was admitted on {} but its endpoint locator is unavailable; refusing PID fallback",
                session.pubkey,
                kind.as_str()
            )
        }
        HostedEndpoint::Unhosted => terminate_unhosted(session).await,
    }
}

pub(super) async fn terminate_explicit_unbound_pty(endpoint_id: &str) -> Result<Option<String>> {
    if !crate::pty::read_all_metadata()
        .iter()
        .any(|metadata| metadata.id == endpoint_id)
    {
        return Ok(None);
    }
    let transport = crate::session_host::transport::transport_for_kind(TransportKind::Pty);
    let endpoint = crate::session_host::transport::EndpointRef {
        kind: TransportKind::Pty,
        endpoint_id: endpoint_id.to_string(),
    };
    if transport.is_live(&endpoint) {
        transport
            .kill(&endpoint)
            .await
            .with_context(|| format!("killing unbound PTY endpoint {endpoint_id}"))?;
        wait_for_process_exit(|| !transport.is_live(&endpoint)).await?;
    } else if !crate::pty::terminate_explicit_owned_supervisor(endpoint_id)? {
        return Ok(None);
    }
    Ok(Some(format!("pty={endpoint_id}")))
}

pub(super) async fn shutdown_daemon_owned_rpc_sessions(state: &Arc<DaemonState>) {
    shutdown::daemon_owned_rpc_sessions(state).await;
}

pub(super) fn record_control_unavailable(
    state: &Arc<DaemonState>,
    session: &Session,
    at: u64,
) -> Result<bool> {
    state.with_store(|store| match session.runtime_state {
        RuntimeState::Running => store.mark_session_presentation_unavailable(
            &session.pubkey,
            session.runtime_generation,
            session.attachment_epoch,
            at,
        ),
        RuntimeState::Stopping => store.cancel_idle_eviction_on_presentation_change(
            &session.pubkey,
            session.runtime_generation,
            session.lifecycle_epoch,
            session.attachment_epoch,
            crate::state::PresentationState::Unavailable,
            at,
        ),
        RuntimeState::Stopped => Ok(false),
    })
}

async fn terminate_explicit_pty(
    transport: &crate::session_host::transport::TransportImpl,
    endpoint: &crate::session_host::transport::EndpointRef,
    session: &Session,
) -> Result<()> {
    if transport.is_live(endpoint)
        && transport.kill(endpoint).await.is_ok()
        && wait_for_process_exit(|| !transport.is_live(endpoint))
            .await
            .is_ok()
    {
        return Ok(());
    }
    match crate::pty::terminate_explicit_owned_supervisor(&endpoint.endpoint_id)? {
        true => Ok(()),
        false if tracked_process_alive(session) => anyhow::bail!(
            "PTY endpoint {:?} is unreachable and has no exact owned-supervisor metadata",
            endpoint.endpoint_id
        ),
        false => Ok(()),
    }
}

async fn terminate_unhosted(session: &Session) -> Result<String> {
    let Some(pid) = session.child_pid else {
        anyhow::bail!(
            "runtime generation {} for {} has no tracked process endpoint",
            session.runtime_generation,
            session.pubkey
        );
    };
    if !crate::liveness::pid_alive(pid) {
        return Ok(format!("pid={pid} (already exited)"));
    }
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        Some(nix::sys::signal::Signal::SIGTERM),
    )
    .with_context(|| format!("sending SIGTERM to pid {pid}"))?;
    wait_for_process_exit(|| !crate::liveness::pid_alive(pid)).await?;
    Ok(format!("pid={pid}"))
}

fn tracked_process_alive(session: &Session) -> bool {
    session.child_pid.is_some_and(crate::liveness::pid_alive)
}

fn clear_runtime_locator(
    state: &Arc<DaemonState>,
    session: &Session,
    locator_kind: &str,
) -> Result<()> {
    state.with_store(|store| {
        store.clear_runtime_locator_if_generation(
            &session.pubkey,
            locator_kind,
            session.runtime_generation,
        )
    })?;
    Ok(())
}

async fn wait_for_process_exit(mut exited: impl FnMut() -> bool) -> Result<()> {
    for _ in 0..100 {
        if exited() {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    anyhow::bail!("process termination was not confirmed within 5 seconds")
}

#[cfg(test)]
#[path = "session_termination/tests.rs"]
mod tests;
