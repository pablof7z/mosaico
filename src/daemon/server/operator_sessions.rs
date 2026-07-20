//! Canonical local-session projection for the unified operator home.

use super::*;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone)]
struct OperatorEndpoint {
    metadata: crate::pty::LaunchMetadata,
    live: bool,
}

const TRANSPORT_PROCESS: &str = "process";
const TRANSPORT_HARNESS: &str = "harness";

pub(super) fn rpc_operator_sessions(state: &Arc<DaemonState>) -> Result<serde_json::Value> {
    let endpoints = crate::pty::read_all_metadata()
        .into_iter()
        .map(|metadata| {
            let live = crate::pty::is_live(&metadata.id);
            (metadata.id.clone(), OperatorEndpoint { metadata, live })
        })
        .collect::<HashMap<_, _>>();
    let sessions = state.with_store(|store| project_sessions(store, &state.host, &endpoints))?;
    Ok(serde_json::json!({ "sessions": sessions }))
}

fn project_sessions(
    store: &Store,
    host: &str,
    endpoints: &HashMap<String, OperatorEndpoint>,
) -> Result<Vec<serde_json::Value>> {
    let channels = store
        .list_channels()?
        .into_iter()
        .map(|channel| (channel.channel_h.clone(), channel))
        .collect::<HashMap<_, _>>();
    let mut rows = Vec::new();
    let mut projected_endpoints = HashSet::new();
    let activities = latest_activities(store)?;
    for rec in store.list_sessions()? {
        let workspaces = session_workspaces(store, &rec.pubkey, &channels)?;
        let (endpoint, transport, takeover) = if rec.is_running() {
            match crate::session_host::transport::hosted_endpoint_for(store, &rec)? {
                crate::session_host::transport::HostedEndpoint::Resolved {
                    transport,
                    endpoint,
                } => {
                    projected_endpoints.insert(endpoint.endpoint_id.clone());
                    let descriptor = transport.describe(&endpoint);
                    let kind = descriptor.kind.as_str();
                    (Some(descriptor), kind, None)
                }
                crate::session_host::transport::HostedEndpoint::Unavailable { kind } => {
                    (None, kind.as_str(), None)
                }
                crate::session_host::transport::HostedEndpoint::Unhosted => {
                    let resumable = rec.child_pid.is_some()
                        && store
                            .native_resume_locator(&rec.pubkey, &rec.observed_harness)?
                            .is_some();
                    (
                        None,
                        if rec.child_pid.is_some() {
                            TRANSPORT_PROCESS
                        } else {
                            TRANSPORT_HARNESS
                        },
                        resumable.then(|| {
                            serde_json::json!({
                                "turn_open": rec.is_working(),
                                "turn_count": rec.turn_count,
                            })
                        }),
                    )
                }
            }
        } else {
            (
                None,
                if rec.admitted_transport.is_empty() {
                    TRANSPORT_HARNESS
                } else {
                    rec.admitted_transport.as_str()
                },
                None,
            )
        };
        let npub = crate::idref::npub(&rec.pubkey)
            .with_context(|| format!("invalid session pubkey {}", rec.pubkey))?;
        let handle = store
            .handle_for_pubkey(&rec.pubkey)?
            .unwrap_or_else(|| rec.agent_slug.clone());
        let resumable = !rec.is_running()
            && rec.recovery_state != crate::state::RecoveryState::Revoked
            && store
                .native_resume_locator(&rec.pubkey, &rec.observed_harness)?
                .is_some();
        let activity = activities
            .get(&rec.pubkey)
            .map(String::as_str)
            .unwrap_or_default();
        rows.push(serde_json::json!({
            "pubkey": rec.pubkey.clone(),
            "npub": npub,
            "handle": handle,
            "agent": rec.agent_slug,
            "title": rec.title,
            "activity": activity,
            "state": if rec.is_running() {
                crate::session_state::SessionState::classify(
                    true,
                    rec.is_working(),
                    crate::session_host::session_has_live_delivery_path(store, &rec),
                )
            } else {
                crate::session_state::SessionState::Offline
            },
            "running": rec.is_running(),
            "resumable": resumable,
            "last_seen": rec.last_seen,
            "host": host,
            "harness": rec.observed_harness,
            "transport": transport,
            "child_pid": rec.child_pid,
            "workspaces": workspaces,
            "endpoint": endpoint,
            "takeover": takeover,
        }));
    }
    let mut unbound = endpoints
        .values()
        .filter(|endpoint| endpoint.live && !projected_endpoints.contains(&endpoint.metadata.id))
        .collect::<Vec<_>>();
    unbound.sort_by(|left, right| left.metadata.id.cmp(&right.metadata.id));
    for endpoint in unbound {
        rows.push(unbound_endpoint_value(store, host, endpoint)?);
    }
    Ok(rows)
}

fn latest_activities(store: &Store) -> Result<HashMap<String, String>> {
    let mut latest = HashMap::<String, (u64, String)>::new();
    for status in store.list_status_sessions(None, None)? {
        let entry = latest
            .entry(status.pubkey)
            .or_insert_with(|| (status.updated_at, status.activity.clone()));
        if status.updated_at > entry.0 {
            *entry = (status.updated_at, status.activity);
        }
    }
    Ok(latest
        .into_iter()
        .map(|(pubkey, (_, activity))| (pubkey, activity))
        .collect())
}

fn session_workspaces(
    store: &Store,
    pubkey: &str,
    channels: &HashMap<String, crate::state::Channel>,
) -> Result<Vec<serde_json::Value>> {
    let mut grouped = BTreeMap::<String, Vec<String>>::new();
    for (channel_id, _) in store.list_session_routes(pubkey)? {
        let root_id = crate::daemon::workspace_path::WorkspacePathResolver::new(store)
            .root_for_channel(&channel_id)?;
        grouped.entry(root_id).or_default().push(channel_id);
    }
    grouped
        .into_iter()
        .map(|(root_id, channel_ids)| workspace_value(store, &root_id, &channel_ids, channels))
        .collect()
}

fn unbound_endpoint_value(
    store: &Store,
    host: &str,
    endpoint: &OperatorEndpoint,
) -> Result<serde_json::Value> {
    let meta = &endpoint.metadata;
    let workspace_name = store
        .get_channel(&meta.root)?
        .as_ref()
        .and_then(crate::state::Channel::human_name)
        .unwrap_or(&meta.root)
        .to_string();
    let workspace_path = crate::daemon::workspace_path::WorkspacePathResolver::new(store)
        .path_for_channel(&meta.root)?
        .or_else(|| Some(meta.cwd.clone()));
    Ok(serde_json::json!({
        "pubkey": "",
        "npub": "",
        "handle": meta.agent,
        "agent": meta.agent,
        "workspaces": [{
            "id": meta.root,
            "name": workspace_name,
            "path": workspace_path,
            "channels": [{"id": meta.root, "name": workspace_name}],
        }],
        "title": meta.command.join(" "),
        "activity": meta.cwd,
        "state": crate::session_state::SessionState::Suspended,
        "running": true,
        "resumable": false,
        "last_seen": 0,
        "host": host,
        "harness": meta.agent,
        "transport": "pty",
        "child_pid": meta.supervisor_pid,
        "bound": false,
        "takeover": null,
        "endpoint": {
            "id": meta.id,
            "kind": "pty",
            "live": endpoint.live,
            "attachable": endpoint.live,
            "cwd": meta.cwd,
            "command": meta.command,
        },
    }))
}

fn workspace_value(
    store: &Store,
    root_id: &str,
    channel_ids: &[String],
    channels: &HashMap<String, crate::state::Channel>,
) -> Result<serde_json::Value> {
    Ok(serde_json::json!({
        "id": root_id,
        "name": channel_name(channels.get(root_id), root_id),
        "path": crate::daemon::workspace_path::WorkspacePathResolver::new(store)
            .path_for_channel(root_id)?,
        "channels": channel_ids
            .iter()
            .map(|id| channel_value(id, channels.get(id)))
            .collect::<Vec<_>>(),
    }))
}

fn channel_value(id: &str, channel: Option<&crate::state::Channel>) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "name": channel_name(channel, id),
    })
}

fn channel_name(channel: Option<&crate::state::Channel>, fallback: &str) -> String {
    channel
        .and_then(crate::state::Channel::human_name)
        .unwrap_or(fallback)
        .to_string()
}

#[cfg(test)]
#[path = "operator_sessions/tests.rs"]
mod tests;
