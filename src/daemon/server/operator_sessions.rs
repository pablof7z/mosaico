//! Canonical local-session projection for the operator session picker.

use super::*;
use std::collections::HashMap;

pub(super) fn rpc_operator_sessions(state: &Arc<DaemonState>) -> Result<serde_json::Value> {
    let metadata = crate::pty::read_all_metadata()
        .into_iter()
        .map(|meta| (meta.id.clone(), meta))
        .collect::<HashMap<_, _>>();
    let sessions = state.with_store(|store| project_sessions(store, &state.host, &metadata))?;
    Ok(serde_json::json!({ "sessions": sessions }))
}

fn project_sessions(
    store: &Store,
    host: &str,
    metadata: &HashMap<String, crate::pty::LaunchMetadata>,
) -> Result<Vec<serde_json::Value>> {
    let channels = store
        .list_channels()?
        .into_iter()
        .map(|channel| (channel.channel_h.clone(), channel))
        .collect::<HashMap<_, _>>();
    let mut rows = Vec::new();
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
        let joined = store
            .list_session_joined_channels(&rec.session_id)?
            .into_iter()
            .map(|(id, _)| channel_value(&id, channels.get(&id)))
            .collect::<Vec<_>>();
        let workspace_id = store
            .root_channel_of(&rec.channel_h)?
            .unwrap_or_else(|| rec.channel_h.clone());
        let workspace_path = store.workspace_path(&workspace_id)?;
        let endpoint = store
            .aliases_for_session(&rec.session_id)?
            .into_iter()
            .find(|alias| alias.external_id_kind == "pty_session")
            .and_then(|alias| metadata.get(&alias.external_id))
            .map(|meta| {
                serde_json::json!({
                    "pty_id": meta.id,
                    "live": crate::pty::is_live(&meta.id),
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
        rows.push(serde_json::json!({
            "session_id": rec.session_id,
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
            "workspace": {
                "id": workspace_id,
                "name": channel_name(channels.get(&workspace_id), &workspace_id),
                "path": workspace_path,
            },
            "channels": joined,
            "endpoint": endpoint,
        }));
    }
    Ok(rows)
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
    fn projection_starts_from_alive_local_sessions_and_keeps_non_pty_rows() {
        let store = Store::open_memory().unwrap();
        store
            .upsert_channel("workspace", "tenex-edge", "", "", 1)
            .unwrap();
        store
            .upsert_channel("room", "review", "", "workspace", 2)
            .unwrap();
        store.upsert_workspace("workspace", "/repo", 3).unwrap();
        let id = store
            .register_session(&crate::state::RegisterSession {
                harness: "codex".into(),
                external_id_kind: "harness_session".into(),
                external_id: "native-1".into(),
                agent_pubkey: "pk-agent".into(),
                agent_slug: "codex".into(),
                channel_h: "room".into(),
                child_pid: Some(42),
                transcript_path: None,
                resume_id: String::new(),
                now: 10,
            })
            .unwrap();

        let rows = project_sessions(&store, "laptop", &HashMap::new()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["session_id"], id);
        assert_eq!(rows[0]["workspace"]["id"], "workspace");
        assert_eq!(rows[0]["workspace"]["path"], "/repo");
        assert_eq!(rows[0]["channels"][0]["name"], "review");
        assert_eq!(rows[0]["transport"], "process");
        assert!(rows[0]["endpoint"].is_null());

        store.mark_dead(&id).unwrap();
        assert!(project_sessions(&store, "laptop", &HashMap::new())
            .unwrap()
            .is_empty());
    }
}
