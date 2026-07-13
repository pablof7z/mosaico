use super::super::*;

#[derive(serde::Deserialize)]
pub(super) struct LaunchPreflightParams {
    agent: String,
    #[serde(default)]
    session_name: Option<String>,
}

pub(crate) fn rpc_agent_launch_preflight(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: LaunchPreflightParams =
        serde_json::from_value(params.clone()).context("agent_launch_preflight params")?;
    if let Some(name) = p.session_name.as_deref().filter(|name| !name.is_empty()) {
        state.with_store(|s| s.ensure_custom_handle_available(&p.agent, name))?;
    }
    let identity =
        crate::identity::load_or_create(&crate::config::edge_home(), &p.agent, now_secs())?;
    let conflict = state.with_store(|s| {
        let session = s.list_alive_sessions()?.into_iter().find(|session| {
            session.agent_slug == p.agent
                && (!identity.per_session_key
                    || s.is_durable_agent_session(&session.session_id)
                        .unwrap_or(false))
        });
        let Some(session) = session else {
            return Ok(None);
        };
        let channels = s
            .list_session_joined_channels(&session.session_id)?
            .into_iter()
            .map(|(channel, _)| channel)
            .collect::<Vec<_>>();
        let pty = s
            .aliases_for_session(&session.session_id)?
            .into_iter()
            .find(|alias| alias.external_id_kind == "pty_session")
            .map(|alias| alias.external_id);
        Ok::<_, anyhow::Error>(Some((session, channels, pty)))
    })?;
    if let Some((session, channels, pty)) = conflict {
        let channels = if channels.is_empty() {
            "<none>".to_string()
        } else {
            channels.join(", ")
        };
        let attach = pty
            .map(|id| format!("tenex-edge pty attach {id}"))
            .unwrap_or_else(|| "tenex-edge mgmt session list".to_string());
        let npub = crate::idref::npub(&identity.pubkey_hex()).unwrap_or(identity.pubkey_hex());
        anyhow::bail!(
            "durable agent {:?} already has live session {} in channel(s) {}; \
             attach with `{attach}`, or add it to another channel with \
             `tenex-edge channel add --session {npub} <channel>`",
            p.agent,
            session.session_id,
            channels
        );
    }
    if identity.per_session_key {
        return Ok(serde_json::json!({ "allowed": true }));
    }
    let reservation = crate::state::mint_session_id();
    state.with_store(|s| {
        s.claim_durable_agent_session(&identity.pubkey_hex(), &p.agent, &reservation, now_secs())
    })?;
    Ok(serde_json::json!({
        "allowed": true,
        "durable_reservation": reservation,
    }))
}

pub(crate) fn rpc_agent_launch_release(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let reservation = params["durable_reservation"]
        .as_str()
        .filter(|value| !value.is_empty())
        .context("agent_launch_release requires durable_reservation")?;
    state.with_store(|s| s.release_durable_agent_session(reservation))?;
    Ok(serde_json::json!({ "released": true }))
}
