use super::{
    admission,
    hosted::{ConversationOpen, HostedOpenRequest, HostedPlacement, HostedPresentation},
    source::resolve_harness_source,
    workspace_abs_path, ResumeRequest,
};
use crate::daemon::server::DaemonState;
use anyhow::Result;
use std::sync::Arc;

/// Resume one exact persisted Mosaico session with its harness-native token.
pub(crate) async fn resume_agent(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    resume_id: &str,
    request: ResumeRequest<'_>,
) -> Result<String> {
    anyhow::ensure!(
        !resume_id.is_empty(),
        "session has no resume token (not resumable)"
    );
    let group = state
        .with_store(|store| store.list_session_routes(&rec.pubkey))?
        .into_iter()
        .map(|(channel, _)| channel)
        .next()
        .unwrap_or_default();
    resume_session_record(state, rec, &rec.work_root, &group, resume_id, request).await
}

/// Resume an exact persisted identity into a caller-selected channel.
pub(crate) async fn resume_agent_in_channel(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    root: &str,
    group: &str,
    resume_id: &str,
    request: ResumeRequest<'_>,
) -> Result<String> {
    anyhow::ensure!(
        !resume_id.is_empty(),
        "session has no resume token (not resumable)"
    );
    resume_session_record(state, rec, root, group, resume_id, request).await
}

async fn resume_session_record(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    root: &str,
    group: &str,
    resume_id: &str,
    request: ResumeRequest<'_>,
) -> Result<String> {
    let mut channels = state
        .with_store(|store| store.list_session_routes(&rec.pubkey))?
        .into_iter()
        .map(|(channel, _)| channel)
        .collect::<Vec<_>>();
    if !group.is_empty() && !channels.iter().any(|channel| channel == group) {
        channels.push(group.to_string());
    }
    channels.sort();
    channels.dedup();
    let harness = crate::session::Harness::from_str(&rec.observed_harness);
    anyhow::ensure!(
        harness != crate::session::Harness::Unknown,
        "session {} has unknown harness {:?}",
        rec.pubkey,
        rec.observed_harness
    );
    let abs_path = workspace_abs_path(state, root, None)?;
    let source = resolve_harness_source(
        harness,
        &rec.agent_slug,
        Some(&rec.admitted_transport),
        request.intent,
    )?;
    let identity = resume_identity(state, rec, harness.as_str())?;
    let preset = source.preset.clone();
    let reservation = admission::reserve_resume_exact(
        state,
        &identity,
        &rec.pubkey,
        &rec.agent_slug,
        harness.as_str(),
        preset.as_deref(),
        source.transport.kind().as_str(),
        root,
        group,
    )?;
    let opened = super::hosted::open(
        state,
        HostedOpenRequest {
            source,
            reservation,
            conversation: ConversationOpen::Resume {
                native_id: resume_id,
            },
            placement: HostedPlacement {
                root,
                abs_path: &abs_path,
                group: Some(group),
                channels: &channels,
            },
            presentation: HostedPresentation {
                ephemeral: false,
                session_name: None,
                dispatch_event: None,
            },
            extra_args: request.extra_args,
        },
    )
    .await?;
    Ok(opened.endpoint.endpoint_id)
}

pub(crate) struct AdoptedNativeSession {
    pub(crate) pty_id: String,
    pub(crate) pubkey: String,
}

pub(crate) async fn adopt_native_session(
    state: &Arc<DaemonState>,
    harness: crate::session::Harness,
    cwd: &std::path::Path,
    root: &str,
    resume_id: &str,
    request: ResumeRequest<'_>,
) -> Result<AdoptedNativeSession> {
    let slug = harness.agent_slug();
    let abs_path = workspace_abs_path(state, root, Some(cwd))?;
    let source = resolve_harness_source(harness, slug, None, request.intent)?;
    let preset = source.preset.clone();
    let reservation = admission::reserve_fresh(
        state,
        &source.identity,
        harness.as_str(),
        preset.as_deref(),
        source.transport.kind().as_str(),
        root,
        Some(root),
        None,
    )?;
    let owner = state.with_store(|store| {
        store.claim_native_resume_locator(
            &reservation.pubkey,
            harness.as_str(),
            resume_id,
            crate::util::now_secs(),
        )
    })?;
    if owner != reservation.pubkey {
        admission::release(state, &reservation);
        anyhow::bail!(
            "native session {resume_id:?} was adopted concurrently by pubkey {owner}; retry"
        );
    }
    let pubkey = reservation.pubkey.clone();
    let channels = vec![root.to_string()];
    let opened = super::hosted::open(
        state,
        HostedOpenRequest {
            source,
            reservation,
            conversation: ConversationOpen::Resume {
                native_id: resume_id,
            },
            placement: HostedPlacement {
                root,
                abs_path: &abs_path,
                group: Some(root),
                channels: &channels,
            },
            presentation: HostedPresentation {
                ephemeral: false,
                session_name: None,
                dispatch_event: None,
            },
            extra_args: request.extra_args,
        },
    )
    .await?;
    Ok(AdoptedNativeSession {
        pty_id: opened.endpoint.endpoint_id,
        pubkey,
    })
}

fn resume_identity(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
    harness: &str,
) -> Result<crate::identity::AgentIdentity> {
    if state.with_store(|store| store.is_derived_session_pubkey(&rec.pubkey))? {
        return Ok(crate::identity::AgentIdentity::per_session(
            &rec.agent_slug,
            harness,
        ));
    }
    crate::identity::load(&crate::config::mosaico_home(), &rec.agent_slug)
}
