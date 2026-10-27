use super::demux::ClaimOutcome;
use super::resolution::work_root_for;
use super::*;

mod admission;
mod running;
use admission::admission;
use channel_membership_rpc::durable_agent;
use running::admit_running_target;

type AddAgentsOp = crate::fabric::nip29::orchestration::AddAgentsOp;

/// One target of an add-agents event that this call durably claimed.
pub(super) struct ClaimedTarget {
    index: usize,
    key: String,
    generation: u32,
}

/// The claim keys this backend's dispatch uses for `op`'s targets addressed
/// to it, with each target's index.
pub(super) fn orchestration_claim_keys(backend_pk: &str, op: &AddAgentsOp) -> Vec<(usize, String)> {
    op.adds
        .iter()
        .enumerate()
        .filter(|(_, target)| target.backend_pubkey == backend_pk)
        .map(|(index, target)| {
            let key = orchestration_target_key(backend_pk, index, target, op.running_only);
            (index, key)
        })
        .collect()
}

/// Durably claim every target of an add-agents orchestration event addressed
/// to THIS backend, before any await. Every returned target must be finished
/// through `run_orchestration`.
pub(super) fn claim_orchestration(
    state: &Arc<DaemonState>,
    event: &Event,
    op: &AddAgentsOp,
) -> Vec<ClaimedTarget> {
    let event_id = event.id.to_hex();
    let signer = event.pubkey.to_hex();
    let mut claimed = Vec::new();
    for (index, key) in orchestration_claim_keys(&state.backend_pubkey(), op) {
        let body = orchestration_target_body(&op.adds[index], op.running_only);
        match state.with_store(|s| {
            s.claim_driven_target(&event_id, &key, &signer, &op.child_h, &body, now_secs())
        }) {
            Ok(Some(generation)) => claimed.push(ClaimedTarget {
                index,
                key,
                generation,
            }),
            Ok(None) => tracing::debug!(
                event_id = %&event_id[..event_id.len().min(8)],
                target = %key,
                "orchestration target already complete, failed, waiting, or in-flight — skipping"
            ),
            Err(e) => tracing::error!(
                event_id = %&event_id[..event_id.len().min(8)],
                target = %key,
                error = %e,
                "orchestration target claim failed"
            ),
        }
    }
    claimed
}

/// Run claimed targets of a subgroup add-agents orchestration event: authorize
/// the signer, confirm the child's declared parent, then either spawn fresh
/// sessions or resume exact prior sessions into the target channel. Every
/// outcome, including a rejection, finishes every claimed target.
pub(super) async fn run_orchestration(
    state: &Arc<DaemonState>,
    event: &Event,
    op: AddAgentsOp,
    claimed: Vec<ClaimedTarget>,
) {
    let event_id = event.id.to_hex();
    if let Some(rejected) = admission(state, event, &op).await {
        for target in &claimed {
            super::demux::finish_claim(state, &event_id, &target.key, target.generation, rejected);
        }
        return;
    }

    let _ = ensure_subscription(state, &op.child_h).await;
    for (position, target) in claimed.iter().enumerate() {
        let queued: Vec<(&str, u32)> = claimed[position + 1..]
            .iter()
            .map(|queued| (queued.key.as_str(), queued.generation))
            .collect();
        let add = &op.adds[target.index];
        let work = async {
            if op.running_only {
                admit_running_target(state, &op, add).await.into()
            } else if add.session_pubkey.as_deref().is_some_and(|s| !s.is_empty()) {
                resume_target(state, &op, add).await.into()
            } else if let Some(rec) = durable_agent::running_durable_session(state, &add.slug) {
                durable_agent::admit_for_orchestration(state, &rec, &op.child_h)
                    .await
                    .into()
            } else {
                spawn_target(state, &op, add).await
            }
        };
        super::demux::run_owned(
            state,
            &event_id,
            &target.key,
            target.generation,
            &queued,
            work,
        )
        .await;
    }
}

fn orchestration_target_key(
    backend_pk: &str,
    target_index: usize,
    target: &crate::fabric::nip29::orchestration::AddTarget,
    running_only: bool,
) -> String {
    let session = target.session_pubkey.as_deref().unwrap_or("");
    let base = format!(
        "{}{backend_pk}:{target_index}:{}:{session}",
        crate::state::ORCHESTRATION_CLAIM_PREFIX,
        target.slug
    );
    if running_only {
        format!("{base}:running")
    } else {
        base
    }
}

fn orchestration_target_body(
    target: &crate::fabric::nip29::orchestration::AddTarget,
    running_only: bool,
) -> String {
    if running_only {
        return format!(
            "admit-running {} {}",
            target.slug,
            target.session_pubkey.as_deref().unwrap_or("")
        );
    }
    match target.session_pubkey.as_deref().filter(|s| !s.is_empty()) {
        Some(session) => format!("resume {} {session}", target.slug),
        None => format!("spawn {}", target.slug),
    }
}

async fn resume_target(
    state: &Arc<DaemonState>,
    op: &crate::fabric::nip29::orchestration::AddAgentsOp,
    target: &crate::fabric::nip29::orchestration::AddTarget,
) -> bool {
    let session_pubkey = target.session_pubkey.as_deref().unwrap_or_default();
    let rec = match state
        .with_store(|s| s.get_session(session_pubkey))
        .ok()
        .flatten()
    {
        Some(rec) => rec,
        None => {
            tracing::warn!(
                session_pubkey,
                child = %op.child_h,
                "orchestration resume target is not known on this backend"
            );
            return false;
        }
    };
    let Some(resume_id) = super::pty_rpc::resume_token_for(state, &rec) else {
        tracing::warn!(
            pubkey = %rec.pubkey,
            child = %op.child_h,
            "orchestration resume target has no harness resume token"
        );
        return false;
    };
    let work_root = match state.with_store(|store| work_root_for(store, &op.child_h)) {
        Ok(root) => root,
        Err(error) => {
            tracing::error!(child = %op.child_h, %error, "orchestration resume workspace lookup failed");
            return false;
        }
    };
    match crate::session_host::resume_agent_in_channel(
        state,
        &rec,
        &work_root,
        &op.child_h,
        &resume_id,
        crate::session_host::ResumeRequest::without_args(
            crate::session_host::LaunchIntent::Managed,
        ),
    )
    .await
    {
        Ok(pty_id) => {
            tracing::info!(
                pubkey = %rec.pubkey,
                slug = %rec.agent_slug,
                child = %op.child_h,
                pty_id = %pty_id,
                "orchestration: session resumed"
            );
            true
        }
        Err(e) => {
            tracing::error!(
                pubkey = %rec.pubkey,
                slug = %rec.agent_slug,
                child = %op.child_h,
                error = %e,
                "orchestration: session resume failed"
            );
            false
        }
    }
}

async fn spawn_target(
    state: &Arc<DaemonState>,
    op: &crate::fabric::nip29::orchestration::AddAgentsOp,
    target: &crate::fabric::nip29::orchestration::AddTarget,
) -> ClaimOutcome {
    let slug = &target.slug;
    let work_root = match state.with_store(|s| work_root_for(s, &op.child_h)) {
        Ok(root) => root,
        Err(error) => {
            tracing::error!(child = %op.child_h, %error, "orchestration spawn workspace lookup failed");
            return ClaimOutcome::Retry("orchestration workspace lookup failed");
        }
    };
    match crate::session_host::spawn_ephemeral_agent(
        state,
        slug,
        &work_root,
        Some(&op.child_h),
        None,
    )
    .await
    {
        Ok(endpoint) => {
            tracing::info!(slug = %slug, child = %op.child_h, endpoint = %endpoint.endpoint_id, "orchestration: agent spawned");
            ClaimOutcome::Completed
        }
        Err(e) => {
            tracing::error!(slug = %slug, child = %op.child_h, error = %e, "orchestration: agent spawn failed");
            if crate::session_host::UnknownAgent::is_cause_of(&e) {
                ClaimOutcome::Failed("unknown agent slug")
            } else {
                ClaimOutcome::Retry("agent spawn failed")
            }
        }
    }
}
