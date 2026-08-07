//! Opening the durable store, and saying which refusal it was when NMP says no.
//!
//! This is the daemon's first irreversible commitment and the point it dies
//! before it can answer a single RPC, so the named condition has to reach the
//! daemon log from here — otherwise the operator has a daemon that will not
//! start and no way to tell a superseded schema epoch from a failing disk.
//! Mosaico deletes nothing on its own: exactly one of those conditions is fixed
//! by discarding the store, and the discard is a command a person types
//! (`mosaico daemon discard-superseded-store`).

use anyhow::Result;

use crate::daemon::storage_paths::StoragePaths;
use crate::nmp_host::{store::StoreCondition, NmpHost};

pub(in crate::daemon::server) fn open(
    cfg: &crate::config::Config,
    storage: &StoragePaths,
    backend_keys: &nostr::Keys,
) -> Result<NmpHost> {
    let error = match NmpHost::open(
        &cfg.relays,
        Some(&cfg.indexer_relay),
        Some(&storage.nmp_store_path),
        backend_keys,
    ) {
        Ok(host) => return Ok(host),
        Err(error) => error,
    };
    if let Some(condition) = StoreCondition::of_open_error(&error) {
        tracing::error!(
            store = %storage.nmp_store_path.display(),
            condition = condition.state(),
            summary = condition.summary(),
            fix = condition.remedy(),
            "NMP refused the durable store; this daemon cannot start"
        );
    }
    Err(error)
}
