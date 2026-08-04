use super::*;

pub(in crate::daemon::server) async fn rpc_doctor(
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value> {
    let relays = state.cfg.relays.clone();
    let probe = state
        .keys_for(&state.hosted_pubkeys().first().cloned().unwrap_or_default())
        .map(|k| k.public_key().to_hex());
    let write_probe = state.provider.doctor_probe().await;
    let background_writes = state.nmp.background_write_snapshot();
    Ok(serde_json::json!({
        "storage": crate::daemon::storage_paths::StoragePaths::current(),
        "relays": relays,
        "probe_pubkey": probe,
        "write_probe": write_probe,
        "background_writes": background_writes,
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

/// Wait for the channel's relay-signed roster to be present in the cache.
///
/// NOT a fetch. The roster arrives on the ONE retained group-records
/// observation, so the only thing an RPC that has just mutated membership can
/// honestly do is wait for that observation to deliver. Waiting is bounded;
/// `false` means "not observed within the window", never "no such roster".
pub(in crate::daemon::server) async fn refresh_channel_members_cache(
    state: &Arc<DaemonState>,
    channel: &str,
) -> bool {
    const ATTEMPTS: u32 = 10;
    for attempt in 0..ATTEMPTS {
        if state.with_store(|s| s.has_channel_membership_snapshot(channel).unwrap_or(false)) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100 * (attempt as u64 + 1).min(5))).await;
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

#[cfg(test)]
#[path = "diagnostics/tests.rs"]
mod tests;

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
