use super::*;
use crate::reconcile::session_start::{plan_from_request, SessionStartAction};
use crate::reconcile::{
    InputFact, SessionStartCommand, SessionStartFailedFact, SessionStartRequestFact,
};

pub(super) fn observed_command(req: &SessionStartRequestFact) -> SessionStartCommand {
    SessionStartCommand {
        session_id: req.session_id.clone(),
        action: if req.already_running {
            SessionStartAction::Reassert
        } else {
            SessionStartAction::Execute
        },
        plan: plan_from_request(req),
        failure_stage: None,
        failure_error: None,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn request_fact(
    session_id: &str,
    agent: &str,
    harness: &str,
    external_id_kind: &str,
    external_id: String,
    native_id: &str,
    work_root: &str,
    channel_h: &str,
    channel_for_upsert: String,
    rel_cwd: &str,
    room_parent: Option<String>,
    watch_pid: Option<i32>,
    tmux_pane: Option<String>,
    ring_doorbell: bool,
    base_pubkey: String,
    signer_pubkey: String,
    signer_label: String,
    signer_ordinal: u32,
    already_running: bool,
    channel_already_subscribed: bool,
    at: u64,
) -> SessionStartRequestFact {
    SessionStartRequestFact {
        session_id: session_id.to_string(),
        agent: agent.to_string(),
        harness: harness.to_string(),
        external_id_kind: external_id_kind.to_string(),
        external_id,
        native_id: native_id.to_string(),
        work_root: work_root.to_string(),
        channel_h: channel_h.to_string(),
        channel_for_upsert,
        rel_cwd: rel_cwd.to_string(),
        room_parent,
        watch_pid,
        tmux_pane,
        ring_doorbell,
        base_pubkey,
        signer_pubkey,
        signer_label,
        signer_ordinal,
        already_running,
        channel_already_subscribed,
        at,
    }
}

pub(super) fn drive_request(
    state: &Arc<DaemonState>,
    req: SessionStartRequestFact,
    observed: &SessionStartCommand,
) -> Result<SessionStartCommand> {
    let command = drive_fact(
        state,
        "session_start_requested",
        InputFact::SessionStartRequested(req.clone()),
    )?
    .ok_or_else(|| anyhow::anyhow!("session_start advisory emitted no staged intent"))?;
    let matched = &command == observed;
    if matched {
        tracing::debug!(
            session = %req.session_id,
            shadow_matches = 1,
            shadow_total = 1,
            "session_start advisory shadow comparison matched"
        );
    } else {
        tracing::warn!(
            session = %req.session_id,
            observed = ?observed,
            derived = ?command,
            "session_start advisory shadow comparison mismatch"
        );
    }
    Ok(command)
}

pub(super) fn record_started(
    state: &Arc<DaemonState>,
    session_id: &str,
    channel_h: &str,
    agent_pubkey: &str,
    pid: Option<i32>,
    at: u64,
) {
    if let Err(e) = drive_fact(
        state,
        "session_started",
        InputFact::SessionStarted {
            session_id: session_id.to_string(),
            channel_h: Some(channel_h.to_string()),
            agent_pubkey: Some(agent_pubkey.to_string()),
            pid,
            at,
        },
    ) {
        tracing::warn!(
            session = %session_id,
            error = %e,
            "failed to record session_start success fact"
        );
    }
}

pub(super) fn record_failed(
    state: &Arc<DaemonState>,
    session_id: &str,
    stage: &str,
    error: &anyhow::Error,
    at: u64,
) {
    if let Err(e) = drive_fact(
        state,
        "session_start_failed",
        InputFact::SessionStartFailed(SessionStartFailedFact {
            session_id: session_id.to_string(),
            stage: stage.to_string(),
            error: format!("{error:#}"),
            at,
        }),
    ) {
        tracing::warn!(
            session = %session_id,
            stage,
            error = %e,
            "failed to record session_start failure fact"
        );
    }
}

fn drive_fact(
    state: &Arc<DaemonState>,
    trigger: &str,
    fact: InputFact,
) -> Result<Option<SessionStartCommand>> {
    let start = std::time::Instant::now();
    let facts = vec![fact.clone()];
    let trigger_ref = session_id(&fact).map(str::to_string);
    let (preview, outcome, commit) = {
        let mut r = state
            .session_start
            .lock()
            .expect("session_start mutex poisoned");
        let preview = r
            .preview_fact(&fact)
            .map_err(|e| anyhow::anyhow!("session_start preview failed: {e:?}"))?
            .ok_or_else(|| anyhow::anyhow!("unsupported session_start fact"))?;
        let outcome = r
            .drive(fact)
            .map_err(|e| anyhow::anyhow!("session_start drive failed: {e:?}"))?;
        let mut commit = crate::reconcile::CommitFacts::from_result(
            r.labels(),
            &outcome.result,
            r.graph_node_count(),
        );
        commit.graph_resources = r.state_rows().len() as i64;
        (preview.result, outcome, commit)
    };
    if !crate::reconcile::preview::command_plans_match(
        preview.resource_plan.commands(),
        outcome.result.resource_plan.commands(),
    ) {
        anyhow::bail!("session_start advisory blocked: committed plan was not previewed first");
    }
    let created_at = crate::instrument::now_millis();
    let duration_us = start.elapsed().as_micros() as i64;
    state.with_store(|s| {
        crate::instrument::record_commit(
            s,
            "session_start",
            trigger,
            trigger_ref.as_deref(),
            &commit,
            duration_us,
            created_at,
        );
        crate::replay_capsules::record_many(
            s,
            "session_start",
            trigger,
            trigger_ref.as_deref(),
            facts,
            created_at,
        );
    });
    Ok(outcome.command)
}

fn session_id(fact: &InputFact) -> Option<&str> {
    match fact {
        InputFact::SessionStartRequested(req) => Some(req.session_id.as_str()),
        InputFact::SessionStarted { session_id, .. } => Some(session_id.as_str()),
        InputFact::SessionStartFailed(failed) => Some(failed.session_id.as_str()),
        _ => None,
    }
}
