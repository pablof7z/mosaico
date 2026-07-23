use super::*;

#[allow(clippy::too_many_arguments)]
pub(in crate::daemon::server) fn engine_params_for(
    cfg: &Config,
    identity: crate::identity::SessionIdentity,
    keys: Keys,
    runtime_generation: u64,
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
        runtime_generation,
        host: cfg.host.clone(),
        rel_cwd: rel_cwd.to_string(),
        dispatch_event,
        owners: cfg.whitelisted_pubkeys.clone(),
        relays: cfg.relays.clone(),
        watch_pid,
        store_path: store_path(),
        presence_lease_interval: env_duration(
            "MOSAICO_PRESENCE_LEASE_MS",
            Duration::from_secs(crate::domain::PRESENCE_LEASE_RENEWAL_SECS),
        ),
        process_probe_interval: env_duration("MOSAICO_PROCESS_PROBE_MS", Duration::from_secs(5)),
    }
}
