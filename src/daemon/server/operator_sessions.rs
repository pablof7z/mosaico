//! Canonical local-session projection for the operator session picker.

use super::*;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone)]
struct OperatorEndpoint {
    metadata: crate::pty::LaunchMetadata,
    live: bool,
}

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
    for rec in store.list_alive_sessions()? {
        let identity = store
            .session_identity_for_session(&rec.session_id)?
            .unwrap_or_else(|| {
                crate::identity::SessionIdentity::fallback(
                    &rec.session_id,
                    rec.agent_slug.clone(),
                    rec.agent_pubkey.clone(),
                )
            });
        let mut grouped = BTreeMap::<String, Vec<String>>::new();
        for (channel_id, _) in store.list_session_joined_channels(&rec.session_id)? {
            let root_id = store
                .root_channel_of(&channel_id)?
                .unwrap_or_else(|| channel_id.clone());
            grouped.entry(root_id).or_default().push(channel_id);
        }
        let workspaces = grouped
            .into_iter()
            .map(|(root_id, channel_ids)| workspace_value(store, &root_id, &channel_ids, &channels))
            .collect::<Result<Vec<_>>>()?;
        let endpoint = store
            .aliases_for_session(&rec.session_id)?
            .into_iter()
            .find(|alias| alias.external_id_kind == "pty_session")
            .and_then(|alias| endpoints.get(&alias.external_id))
            .map(|endpoint| {
                let meta = &endpoint.metadata;
                projected_endpoints.insert(meta.id.clone());
                serde_json::json!({
                    "pty_id": meta.id,
                    "live": endpoint.live,
                    "cwd": meta.cwd,
                    "command": meta.command,
                })
            });
        let transport = if endpoint.is_some() {
            "pty"
        } else if rec.child_pid.is_some() {
            "process"
        } else {
            "harness"
        };
        let npub = crate::idref::npub(&rec.agent_pubkey)
            .with_context(|| format!("invalid session pubkey {}", rec.agent_pubkey))?;
        rows.push(serde_json::json!({
            "pubkey": rec.agent_pubkey.clone(),
            "npub": npub,
            "handle": identity.display_slug(),
            "agent": rec.agent_slug,
            "title": rec.title,
            "activity": rec.activity,
            "busy": rec.working,
            "last_seen": rec.last_seen,
            "host": host,
            "harness": rec.harness,
            "transport": transport,
            "child_pid": rec.child_pid,
            "workspaces": workspaces,
            "endpoint": endpoint,
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
    let workspace_path = store
        .workspace_path(&meta.root)?
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
        "busy": false,
        "last_seen": 0,
        "host": host,
        "harness": meta.agent,
        "transport": "pty",
        "child_pid": meta.supervisor_pid,
        "bound": false,
        "endpoint": {
            "pty_id": meta.id,
            "live": endpoint.live,
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
        "path": store.workspace_path(root_id)?,
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
mod tests {
    use super::*;

    #[test]
    fn projection_exposes_public_identity_without_the_private_runtime_id() {
        use nostr_sdk::prelude::Keys;

        let store = Store::open_memory().unwrap();
        store
            .upsert_channel("workspace", "tenex-edge", "", "", 1)
            .unwrap();
        store
            .upsert_channel("room", "review", "", "workspace", 2)
            .unwrap();
        store.upsert_workspace("workspace", "/repo", 3).unwrap();
        store
            .upsert_channel("skills-root", "skills", "", "", 3)
            .unwrap();
        store
            .upsert_channel("skill-dev", "skill-dev", "", "skills-root", 4)
            .unwrap();
        store.upsert_workspace("skills-root", "/skills", 4).unwrap();
        let pubkey = Keys::generate().public_key().to_hex();
        let id = store
            .register_session(&crate::state::RegisterSession {
                harness: "codex".into(),
                external_id_kind: "harness_session".into(),
                external_id: "native-1".into(),
                agent_pubkey: pubkey.clone(),
                agent_slug: "codex".into(),
                channel_h: "room".into(),
                child_pid: Some(42),
                transcript_path: None,
                resume_id: String::new(),
                now: 10,
            })
            .unwrap();
        store.join_session_channel(&id, "skill-dev", 11).unwrap();

        let rows = project_sessions(&store, "laptop", &HashMap::new()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["pubkey"], pubkey);
        assert_eq!(
            rows[0]["npub"],
            crate::idref::npub(&pubkey).expect("valid npub")
        );
        assert!(rows[0].get("session_id").is_none());
        assert_eq!(rows[0]["workspaces"].as_array().unwrap().len(), 2);
        assert_eq!(rows[0]["workspaces"][0]["id"], "skills-root");
        assert_eq!(rows[0]["workspaces"][0]["path"], "/skills");
        assert_eq!(rows[0]["workspaces"][0]["channels"][0]["name"], "skill-dev");
        assert_eq!(rows[0]["workspaces"][1]["id"], "workspace");
        assert_eq!(rows[0]["workspaces"][1]["path"], "/repo");
        assert_eq!(rows[0]["workspaces"][1]["channels"][0]["name"], "review");
        assert_eq!(rows[0]["transport"], "process");
        assert!(rows[0]["endpoint"].is_null());

        store.mark_dead(&id).unwrap();
        assert!(project_sessions(&store, "laptop", &HashMap::new())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn projection_includes_live_unbound_supervisor() {
        let store = Store::open_memory().unwrap();
        store
            .upsert_channel("workspace", "tenex-edge", "", "", 1)
            .unwrap();
        store.upsert_workspace("workspace", "/repo", 1).unwrap();
        let metadata = crate::pty::LaunchMetadata {
            id: "pty-1".into(),
            socket: "/tmp/pty-1.sock".into(),
            supervisor_pid: 42,
            agent: "codex".into(),
            root: "workspace".into(),
            cwd: "/repo/subdir".into(),
            ephemeral: false,
            command: vec!["codex".into(), "--yolo".into()],
        };
        let endpoints = HashMap::from([(
            metadata.id.clone(),
            OperatorEndpoint {
                metadata,
                live: true,
            },
        )]);

        let rows = project_sessions(&store, "laptop", &endpoints).unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["bound"], false);
        assert_eq!(rows[0]["handle"], "codex");
        assert_eq!(rows[0]["endpoint"]["pty_id"], "pty-1");
        assert_eq!(rows[0]["workspaces"][0]["name"], "tenex-edge");
        assert_eq!(rows[0]["title"], "codex --yolo");
    }
}
