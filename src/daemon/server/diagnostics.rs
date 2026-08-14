use super::*;

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
    Ok(serde_json::json!({ "pubkey": pubkey, "backend_label": state.host().clone() }))
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
