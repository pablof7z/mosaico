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
        "trellis": super::probe::doctor_summary(state)?,
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
        .backend_pubkey()
        .ok_or_else(|| anyhow::anyhow!("no signing key (tenexPrivateKey) configured"))?;
    Ok(serde_json::json!({ "pubkey": pubkey, "backend_label": state.host.clone() }))
}

pub(in crate::daemon::server) async fn refresh_channel_members_cache(
    state: &Arc<DaemonState>,
    channel: &str,
) -> bool {
    use crate::fabric::nip29::materializer::Nip29Materializer;
    use crate::fabric::nip29::wire::{kind, KIND_GROUP_ADMINS, KIND_GROUP_MEMBERS};
    use nostr_sdk::prelude::Filter;

    let filter = Filter::new()
        .kinds([kind(KIND_GROUP_ADMINS), kind(KIND_GROUP_MEMBERS)])
        .identifier(channel)
        .limit(10);
    let Ok(events) = state.transport.fetch(filter, Duration::from_secs(5)).await else {
        return false;
    };
    let newest_admins = events
        .iter()
        .filter(|e| e.kind.as_u16() == KIND_GROUP_ADMINS)
        .max_by_key(|e| e.created_at.as_secs());
    let newest_members = events
        .iter()
        .filter(|e| e.kind.as_u16() == KIND_GROUP_MEMBERS)
        .max_by_key(|e| e.created_at.as_secs());
    if newest_admins.is_none() && newest_members.is_none() {
        return false;
    }
    state.with_store(|s| {
        if let Some(ev) = newest_admins {
            Nip29Materializer::materialize_admins(s, ev);
        }
        if let Some(ev) = newest_members {
            Nip29Materializer::materialize_members(s, ev);
        }
    });
    true
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
    // Debug dump: show every pending row regardless of retry backoff window.
    let rows = state.with_store(|s| s.peek_outbox(p.limit as u32, u64::MAX))?;
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

/// `explain <handle>`: resolve a `scheme:value` handle against the receipts +
/// llm_calls ledgers. The store is daemon-owned, so the CLI reaches the pure
/// [`crate::explain`] engine through this one RPC (like `who`).
pub(in crate::daemon::server) fn rpc_explain(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let handle = params
        .get("handle")
        .and_then(|h| h.as_str())
        .context("explain: missing `handle` param")?;
    let handle = crate::explain::parse_handle(handle)?;
    state.with_store(|s| crate::explain::explain(s, &handle))
}
