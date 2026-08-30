//! Opening the durable store, and saying which refusal it was when Fava says no.
//!
//! This is the daemon's first irreversible commitment and the point it dies
//! before it can answer a single RPC, so the named condition has to reach the
//! daemon log from here — otherwise the operator has a daemon that will not
//! start and no way to tell a superseded schema epoch from a failing disk.
//! Mosaico deletes nothing on its own: exactly one condition offers the
//! explicit, config-preserving full-state reset a person must confirm.

use anyhow::Result;

use crate::daemon::storage_paths::StoragePaths;
use crate::fava_host::{store::StoreCondition, FavaHost};

pub(in crate::daemon::server) fn open(
    cfg: &crate::config::Config,
    storage: &StoragePaths,
    backend_keys: &nostr::key::Keys,
) -> Result<FavaHost> {
    let error = match FavaHost::open(
        &cfg.relays,
        Some(&cfg.indexer_relay),
        Some(&storage.fava_store_path),
        backend_keys,
    ) {
        Ok(host) => return Ok(host),
        Err(error) => error,
    };
    if let Some(condition) = StoreCondition::of_open_error(&error) {
        tracing::error!(
            store = %storage.fava_store_path.display(),
            condition = condition.state(),
            summary = condition.summary(),
            fix = condition.remedy(),
            "Fava refused the durable store; this daemon cannot start"
        );
    }
    Err(error)
}
