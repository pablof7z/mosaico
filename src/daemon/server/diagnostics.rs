use super::*;

pub(in crate::daemon::server) async fn rpc_doctor(
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value> {
    let relays = state.cfg.relays.clone();
    let probe = state
        .keys_for(&state.hosted_pubkeys().first().cloned().unwrap_or_default())
        .map(|k| k.public_key().to_hex());
    // The probe's wire shape lives in the provider; readers only see strings.
    let (publish, readback) = state.provider.doctor_probe().await;
    Ok(serde_json::json!({
        "storage": crate::daemon::storage_paths::StoragePaths::current(),
        "relays": relays,
        "probe_pubkey": probe,
        "publish": publish,
        "readback": readback,
    }))
}

// ── local_backend ────────────────────────────────────────────────────────────

/// Return the local daemon's backend pubkey and exact config `backendName` label
/// so callers can construct `slug@backend-label` agent specs without guessing
/// or deriving any machine hostname.
pub(in crate::daemon::server) fn rpc_local_backend(
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value> {
    let pubkey = state
        .backend_pubkey
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no signing key (tenexPrivateKey) configured"))?;
    Ok(serde_json::json!({ "pubkey": pubkey, "backend_label": state.host.clone() }))
}

pub(in crate::daemon::server) async fn refresh_project_members_cache(
    state: &Arc<DaemonState>,
    project: &str,
) -> bool {
    use crate::fabric::nip29::wire::{kind, KIND_GROUP_MEMBERS};
    use nostr_sdk::prelude::Filter;

    let filter = Filter::new()
        .kind(kind(KIND_GROUP_MEMBERS))
        .identifier(project)
        .limit(5);
    let Ok(events) = state.transport.fetch(filter, Duration::from_secs(5)).await else {
        return false;
    };
    let Some(ev) = events.iter().max_by_key(|e| e.created_at.as_secs()) else {
        return false;
    };
    let mut admins: Vec<String> = Vec::new();
    let mut members: Vec<String> = Vec::new();
    for t in ev.tags.iter() {
        let s = t.as_slice();
        if s.first().map(String::as_str) != Some("p") {
            continue;
        }
        let Some(pubkey) = s.get(1).cloned() else {
            continue;
        };
        let role = s.get(2).map(String::as_str).unwrap_or("member");
        if role == "admin" {
            admins.push(pubkey);
        } else {
            members.push(pubkey);
        }
    }
    let now = now_secs();
    state.with_store(|s| {
        s.replace_channel_admins(project, &admins, now).ok();
        s.replace_channel_members(project, &members, now).ok();
    });
    true
}

pub(in crate::daemon::server) async fn wait_for_project_member_cache(
    state: &Arc<DaemonState>,
    project: &str,
    pubkey: &str,
    present: bool,
) -> bool {
    for _ in 0..20 {
        let refreshed = refresh_project_members_cache(state, project).await;
        let has = state.with_store(|s| s.is_channel_member(project, pubkey).unwrap_or(false));
        if refreshed && has == present {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

pub(in crate::daemon::server) fn log_nip29_role_decision(
    group: &str,
    pubkey: &str,
    role: &str,
    reason: &str,
) {
    tracing::debug!(
        group,
        target = %crate::util::pubkey_short(pubkey),
        role,
        reason,
        "nip29 role decision"
    );
}

pub(in crate::daemon::server) fn rpc_debug_outbox(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        #[serde(default = "default_debug_outbox_limit")]
        limit: u64,
    }
    let p: P = serde_json::from_value(params.clone()).unwrap_or(P {
        limit: default_debug_outbox_limit(),
    });
    // The outbox is now a generic signed-event publish queue (raw event_json),
    // not status-specific snapshots — dump the pending queue rows verbatim.
    let rows = state.with_store(|s| s.drain_outbox(p.limit as u32))?;
    let rows = rows
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "local_id": r.local_id,
                "state": r.state,
                "retries": r.retries,
                "last_error": r.last_error,
                "enqueued_at": r.enqueued_at,
                "event_json": r.event_json,
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({ "rows": rows }))
}

pub(in crate::daemon::server) fn default_debug_outbox_limit() -> u64 {
    50
}
