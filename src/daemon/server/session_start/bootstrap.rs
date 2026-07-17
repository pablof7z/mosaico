use super::*;

pub(crate) struct HostedSessionStart<'a> {
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

pub(crate) async fn bootstrap_hosted_session_start(
    state: &Arc<DaemonState>,
    endpoint: &crate::session_host::transport::SessionEndpoint,
    request: HostedSessionStart<'_>,
) -> Result<String> {
    let meta = &endpoint.meta;
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
            "watch_pid": endpoint.watch_pid,
            "pty_session": &endpoint.endpoint_id,
            "endpoint_kind": endpoint.kind,
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
