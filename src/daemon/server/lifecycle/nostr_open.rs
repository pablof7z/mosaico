//! Validate and construct the daemon's sole Nostr runtime.
//!
//! This is the daemon's first irreversible commitment and the point it dies
//! before it can answer a single RPC, so the named condition has to reach the
//! daemon log from here — otherwise the operator has a daemon that will not
//! start and no way to tell a superseded schema epoch from a failing disk.
//! Mosaico deletes nothing on its own: exactly one condition offers the
//! explicit, config-preserving full-state reset a person must confirm.

use anyhow::Result;

use crate::nostr_runtime::NostrRuntime;

pub(in crate::daemon::server) fn open(cfg: &crate::config::Config) -> Result<NostrRuntime> {
    NostrRuntime::new(&cfg.relay)
}
