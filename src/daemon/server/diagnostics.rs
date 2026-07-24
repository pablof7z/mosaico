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
        .backend_pubkey()
        .ok_or_else(|| anyhow::anyhow!("no signing key (mosaicoPrivateKey) configured"))?;
    Ok(serde_json::json!({ "pubkey": pubkey, "backend_label": state.host.clone() }))
}

pub(in crate::daemon::server) async fn refresh_channel_members_cache(
    state: &Arc<DaemonState>,
    channel: &str,
) -> bool {
    use crate::fabric::nip29::materializer::Nip29Materializer;
    use crate::fabric::nip29::wire::{KIND_GROUP_ADMINS, KIND_GROUP_MEMBERS};

    let Ok(filter) = crate::nmp_host::read::filter(
        &[KIND_GROUP_ADMINS, KIND_GROUP_MEMBERS],
        &[],
        &[('d', channel.to_string())],
    ) else {
        return false;
    };
    let Ok(events) = state
        .nmp
        .fetch_group(filter, 10, Duration::from_secs(5))
        .await
    else {
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

/// `explain <handle>`: resolve a `scheme:value` handle against the receipts
/// ledger. The store is daemon-owned, so the CLI reaches the pure
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
