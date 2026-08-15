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
    pub(crate) admitted_preset: Option<&'a str>,
    pub(crate) admitted_transport: crate::session_host::transport::TransportKind,
}

pub(crate) async fn bootstrap_hosted_session_start(
    state: &Arc<DaemonState>,
    endpoint: &crate::session_host::transport::SessionEndpoint,
    request: HostedSessionStart<'_>,
) -> Result<String> {
    let meta = &endpoint.meta;
    // Transports that own their native resume token (ACP `sessionId` /
    // app-server thread id) surface it here so it is persisted as the
    // `native_resume` locator at launch. Without this, an ACP/app-server hosted
    // session — which never fires the harness's mosaico hook — would come online
    // with no recorded resume token and silently degrade to a fresh relaunch on
    // restart. The endpoint's own token takes precedence over any prior
    // `resume_id`; they are equal on resume and only the endpoint has it on a
    // fresh launch.
    let resume_id = hosted_resume_id(endpoint.native_id.as_deref(), request.resume_id);
    let response = rpc_session_start(
        state,
        &serde_json::json!({
            "agent": &meta.agent,
            "pubkey": request.pubkey,
            "reclaimed_pubkey": request.reclaimed_pubkey,
            "observed_harness": request.observed_harness.as_str(),
            "admitted_preset": request.admitted_preset,
            "admitted_transport": request.admitted_transport.as_str(),
            "endpoint_provenance": "launch",
            "cwd": &meta.cwd,
            "channel": request.channel,
            "channels": request.channels,
            "watch_pid": endpoint.watch_pid,
            "pty_session": &endpoint.endpoint_id,
            "endpoint_kind": endpoint.kind,
            "resume_id": resume_id,
            "dispatch_event": request.dispatch_event,
            "session_name": request.session_name,
        }),
        None,
    )
    .await?;
    private_run_for_public_response(state, &response)
}

fn hosted_resume_id<'a>(native_id: Option<&'a str>, prior_id: Option<&'a str>) -> Option<&'a str> {
    native_id.or(prior_id)
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

#[cfg(test)]
mod tests {
    use super::hosted_resume_id;

    #[test]
    fn hosted_transport_native_id_is_the_resume_authority() {
        assert_eq!(
            hosted_resume_id(Some("opened-native-id"), None),
            Some("opened-native-id")
        );
        assert_eq!(
            hosted_resume_id(Some("opened-native-id"), Some("prior-native-id")),
            Some("opened-native-id")
        );
        assert_eq!(
            hosted_resume_id(None, Some("hook-native-id")),
            Some("hook-native-id")
        );
    }
}
