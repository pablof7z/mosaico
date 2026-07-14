use super::*;

#[allow(clippy::too_many_arguments)]
pub(in crate::daemon::server) fn engine_params_for(
    cfg: &Config,
    identity: crate::identity::SessionIdentity,
    keys: Keys,
    session_id: &str,
    channel: &str,
    workspace: &str,
    rel_cwd: &str,
    dispatch_event: Option<String>,
    watch_pid: Option<i32>,
) -> EngineParams {
    EngineParams {
        identity,
        keys,
        channel: channel.to_string(),
        workspace: workspace.to_string(),
        session_id: session_id.to_string(),
        host: cfg.host.clone(),
        rel_cwd: rel_cwd.to_string(),
        dispatch_event,
        owners: cfg.whitelisted_pubkeys.clone(),
        relays: cfg.relays.clone(),
        watch_pid,
        store_path: store_path(),
        heartbeat: env_duration("TENEX_EDGE_HEARTBEAT_MS", Duration::from_secs(30)),
        obs_interval: env_duration("TENEX_EDGE_OBS_MS", Duration::from_secs(5)),
        status_ttl: status_ttl_duration(),
        turn_first: Duration::from_secs(env_u64("TENEX_EDGE_TURN_FIRST_S", 30)),
        // 0 = disabled: status re-distill inside one long turn is opt-in.
        turn_repeat: Duration::from_secs(env_u64("TENEX_EDGE_TURN_REPEAT_S", 0)),
    }
}
