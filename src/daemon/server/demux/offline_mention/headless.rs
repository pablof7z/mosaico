use crate::daemon::server::DaemonState;
use std::sync::Arc;

mod notice;

pub(super) struct MentionNotice {
    pub(super) requester_pubkey: Option<String>,
    pub(super) target_label: Option<String>,
}

pub(super) async fn spawn_headless_mention(
    state: &Arc<DaemonState>,
    agent_slug: &str,
    work_root: &str,
    channel_h: &str,
    body: &str,
    resume_id: Option<&str>,
    mention_notice: MentionNotice,
) -> anyhow::Result<bool> {
    if !crate::session_host::agent_supports_headless_exec(agent_slug) {
        return Ok(false);
    }
    let prompt = mention_prompt(body);
    let launch = crate::session_host::spawn_agent_exec(
        state,
        agent_slug,
        work_root,
        &prompt,
        resume_id,
        None,
        Some(channel_h),
        None,
    )
    .await?;
    tracing::info!(
        agent = %agent_slug,
        exec_id = %launch.id,
        pid = launch.pid(),
        log = %launch.log_path.display(),
        "headless agent spawned on mention"
    );
    reap_headless_on_exit(
        state.clone(),
        agent_slug.to_string(),
        channel_h.to_string(),
        mention_notice,
        launch,
    );
    Ok(true)
}

fn reap_headless_on_exit(
    state: Arc<DaemonState>,
    agent_slug: String,
    channel: String,
    mention_notice: MentionNotice,
    launch: crate::session_host::ExecLaunch,
) {
    let crate::session_host::ExecLaunch {
        id,
        mut child,
        log_path,
        started_at,
        harness,
    } = launch;
    let pid = child.id() as i32;
    tokio::spawn(async move {
        let waited = tokio::task::spawn_blocking(move || child.wait()).await;
        let outcome = match waited {
            Ok(Ok(status)) => {
                tracing::info!(
                    agent = %agent_slug,
                    channel = %channel,
                    exec_id = %id,
                    pid,
                    status = %status,
                    log = %log_path.display(),
                    "headless agent exited"
                );
                notice::HeadlessOutcome::Exited {
                    status: status.to_string(),
                    success: status.success(),
                }
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    agent = %agent_slug,
                    channel = %channel,
                    exec_id = %id,
                    pid,
                    error = %e,
                    log = %log_path.display(),
                    "headless agent wait failed"
                );
                notice::HeadlessOutcome::WaitFailed(e.to_string())
            }
            Err(e) => {
                tracing::warn!(
                    agent = %agent_slug,
                    channel = %channel,
                    exec_id = %id,
                    pid,
                    error = %e,
                    log = %log_path.display(),
                    "headless agent wait task failed"
                );
                notice::HeadlessOutcome::WaitTaskFailed(e.to_string())
            }
        };
        crate::session_host::bind_native_id_from_log(&state, &pid.to_string(), &harness, &log_path);
        let session = state.with_store(|s| s.get_session(&pid.to_string()).ok().flatten());
        let session_id = session.as_ref().map(|rec| rec.session_id.clone());
        let has_reply = session
            .as_ref()
            .map(|rec| notice::session_published_reply_since(&state, rec, started_at))
            .unwrap_or(false);
        if !has_reply {
            notice::publish_no_reply_notice(
                &state,
                notice::NoReplyNotice {
                    agent_slug: &agent_slug,
                    channel: &channel,
                    session_id: session_id.as_deref(),
                    requester_pubkey: mention_notice.requester_pubkey.as_deref(),
                    target_label: mention_notice.target_label.as_deref(),
                    exec_id: &id,
                    outcome: &outcome,
                },
            )
            .await;
        }
        if let Err(e) = super::super::super::rpc_session_end(
            &state,
            &serde_json::json!({
                "session": pid.to_string(),
            }),
        )
        .await
        {
            tracing::warn!(
                agent = %agent_slug,
                channel = %channel,
                exec_id = %id,
                pid,
                error = %e,
                "headless agent session_end failed"
            );
        }
    });
}

pub(super) async fn publish_start_failure_notice(
    state: &Arc<DaemonState>,
    agent_slug: &str,
    target_label: &str,
    channel: &str,
    requester_pubkey: Option<&str>,
    detail: &str,
) {
    let outcome = notice::HeadlessOutcome::StartFailed(detail.to_string());
    notice::publish_no_reply_notice(
        state,
        notice::NoReplyNotice {
            agent_slug,
            channel,
            session_id: None,
            requester_pubkey,
            target_label: Some(target_label),
            exec_id: "spawn",
            outcome: &outcome,
        },
    )
    .await;
}

pub(super) fn mention_prompt(body: &str) -> String {
    let body = body.trim();
    let body = if body.is_empty() {
        "You were mentioned in tenex-edge. Check your channel context and respond if needed."
    } else {
        body
    };
    format!(
        "{body}\n\n[reply via `tenex-edge channel send --message \"...\"` - replies do not auto-publish]"
    )
}
