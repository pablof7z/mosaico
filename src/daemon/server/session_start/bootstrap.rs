use super::*;

pub(crate) struct PtySessionStart<'a> {
    pub(crate) pubkey: &'a str,
    pub(crate) reclaimed_pubkey: Option<&'a str>,
    pub(crate) channel: Option<&'a str>,
    pub(crate) channels: &'a [String],
    pub(crate) resume_id: Option<&'a str>,
    pub(crate) dispatch_event: Option<&'a str>,
    pub(crate) session_name: Option<&'a str>,
    pub(crate) observed_harness: Harness,
    pub(crate) admitted_bundle: &'a str,
    pub(crate) admitted_transport: crate::session_host::transport::TransportKind,
}

pub(crate) async fn bootstrap_pty_session_start(
    state: &Arc<DaemonState>,
    meta: &crate::pty::LaunchMetadata,
    request: PtySessionStart<'_>,
) -> Result<String> {
    let watch_pid = i32::try_from(meta.supervisor_pid).ok();
    let response = rpc_session_start(
        state,
        &serde_json::json!({
            "agent": &meta.agent,
            "pubkey": request.pubkey,
            "reclaimed_pubkey": request.reclaimed_pubkey,
            "observed_harness": request.observed_harness.as_str(),
            "admitted_bundle": request.admitted_bundle,
            "admitted_transport": request.admitted_transport.as_str(),
            "endpoint_provenance": "launch",
            "cwd": &meta.cwd,
            "channel": request.channel,
            "channels": request.channels,
            "watch_pid": watch_pid,
            "pty_session": &meta.id,
            "endpoint_kind": if meta.socket.is_empty() { "acp" } else { "pty" },
            "resume_id": request.resume_id,
            "dispatch_event": request.dispatch_event,
            "session_name": request.session_name,
        }),
        None,
    )
    .await?;
    private_run_for_public_response(state, &response)
}

fn private_run_for_public_response(
    state: &Arc<DaemonState>,
    response: &serde_json::Value,
) -> Result<String> {
    let pubkey = response["pubkey"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("session_start bootstrap returned no pubkey"))?;
    state
        .with_store(|store| store.get_session(pubkey))?
        .map(|session| session.pubkey)
        .ok_or_else(|| anyhow::anyhow!("session_start created no runtime for pubkey {pubkey}"))
}
