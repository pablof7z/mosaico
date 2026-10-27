use super::demux::ClaimOutcome;
use super::*;

type SessionDispatchOp = crate::fabric::nip29::session_dispatch::SessionDispatchOp;

/// The claim key this backend's dispatch uses for `op`, or `None` when the
/// dispatch is addressed to another backend.
pub(super) fn session_dispatch_claim_key(
    backend_pk: &str,
    op: &SessionDispatchOp,
) -> Option<String> {
    (op.target.backend_pubkey == backend_pk).then(|| {
        format!(
            "{}{backend_pk}:{}",
            crate::state::SESSION_DISPATCH_CLAIM_PREFIX,
            op.target.slug
        )
    })
}

/// Durably claim a session dispatch addressed to this backend, before any
/// await. `Some((claim_key, generation))` means this call owns the run and
/// must finish it through `run_session_dispatch`.
pub(super) fn claim_session_dispatch(
    state: &Arc<DaemonState>,
    event: &Event,
    op: &SessionDispatchOp,
) -> Option<(String, u32)> {
    let event_id = event.id.to_hex();
    let target_key = session_dispatch_claim_key(&state.backend_pubkey(), op)?;
    let body = format!(
        "dispatch {} in {} on {}",
        op.target.slug,
        op.target.workspace,
        op.target.channels.join(", ")
    );
    let claimed = state.with_store(|s| {
        s.claim_driven_target(
            &event_id,
            &target_key,
            &event.pubkey.to_hex(),
            &op.route_channel,
            &body,
            now_secs(),
        )
    });
    match claimed {
        Ok(Some(generation)) => Some((target_key, generation)),
        Ok(None) => {
            tracing::debug!(event_id = %&event_id[..event_id.len().min(8)], target = %target_key, "session dispatch target already claimed");
            None
        }
        Err(e) => {
            tracing::error!(event_id = %&event_id[..event_id.len().min(8)], error = %e, "session dispatch claim failed");
            None
        }
    }
}

/// Run a claimed session dispatch. Every outcome, including an admission
/// rejection, finishes the claim.
pub(super) async fn run_session_dispatch(
    state: &Arc<DaemonState>,
    event: &Event,
    op: SessionDispatchOp,
    (target_key, generation): (String, u32),
) {
    let event_id = event.id.to_hex();
    if let Some(rejected) = admission(state, event, &op) {
        super::demux::finish_claim(state, &event_id, &target_key, generation, rejected);
        return;
    }
    let work = spawn_dispatched(state, &event_id, &op);
    super::demux::run_owned(state, &event_id, &target_key, generation, &[], work).await;
}

/// `None` admits. The relay-signed roster is read from the cache the ONE
/// retained group-records observation keeps current — no wire call on the
/// dispatch path. `is_channel_member` is true for either list, which is the
/// same admission this used to spell as "named by 39001 or by 39002". A
/// roster that does not (yet) name the signer retries: one group record can
/// arrive before the member lists, so "not a member" is never terminal; the
/// attempt cap bounds a signer who never becomes one.
fn admission(
    state: &Arc<DaemonState>,
    event: &Event,
    op: &SessionDispatchOp,
) -> Option<ClaimOutcome> {
    let signer = event.pubkey.to_hex();
    match state.with_store(|s| s.is_channel_member(&op.route_channel, &signer)) {
        Ok(true) => None,
        Ok(false) => {
            tracing::debug!(
                event_id = %&event.id.to_hex()[..8],
                signer = %crate::util::pubkey_short(&signer),
                channel = %op.route_channel,
                "session dispatch signer is not (yet) a route-channel member"
            );
            Some(ClaimOutcome::Retry(
                "signer is not (yet) a route-channel member",
            ))
        }
        Err(error) => {
            tracing::debug!(event_id = %&event.id.to_hex()[..8], error = %format!("{error:#}"), "route-channel membership lookup failed");
            Some(ClaimOutcome::Retry(
                "route-channel membership could not be verified",
            ))
        }
    }
}

async fn spawn_dispatched(
    state: &Arc<DaemonState>,
    event_id: &str,
    op: &SessionDispatchOp,
) -> ClaimOutcome {
    let slug = &op.target.slug;
    match crate::session_host::spawn_dispatched_ephemeral_agent(
        state,
        slug,
        &op.target.workspace,
        &op.target.channels,
        event_id,
    )
    .await
    {
        Ok(spawned) => {
            tracing::info!(
                slug = %slug,
                workspace = %op.target.workspace,
                endpoint = %spawned.endpoint.endpoint_id,
                pubkey = %spawned.pubkey,
                "session dispatch: agent spawned"
            );
            ClaimOutcome::Completed
        }
        Err(e) => {
            tracing::error!(slug = %slug, workspace = %op.target.workspace, error = %e, "session dispatch: spawn failed");
            if crate::session_host::UnknownAgent::is_cause_of(&e) {
                ClaimOutcome::Failed("unknown agent slug")
            } else {
                ClaimOutcome::Retry("agent spawn failed")
            }
        }
    }
}
