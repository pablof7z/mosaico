//! Why the daemon will not start, read off the durable store itself.
//!
//! A store NMP refuses is a daemon that exits before it can answer an RPC, so
//! every other check here — each of which asks the daemon — reports the
//! symptom. This one asks the store directly and names the condition, which is
//! the only reason an operator can tell a superseded schema epoch from a
//! failing disk without investigating a database file by hand.

use crate::nmp_host::store;

use super::{Check, CheckStatus};

/// Report a daemon that could not be reached, with the store's own account of
/// why when it has one.
///
/// The store is asked first because a store NMP refuses IS this failure. When
/// the store is not the cause the `daemon` check keeps its own fix and this
/// invents no store fault — sending an operator to inspect a healthy file is
/// its own kind of wrong answer.
pub(super) fn diagnose_failed_start(error: &anyhow::Error) -> Vec<Check> {
    let path = crate::daemon::storage_paths::StoragePaths::current().nmp_store_path;
    let mut checks = Vec::new();
    checks.extend(check_for(&path));
    checks.push(
        Check::new(
            "daemon",
            CheckStatus::Error,
            format!("cannot connect or start: {error:#}"),
        )
        .repair(daemon_repair(!checks.is_empty())),
    );
    checks
}

fn check_for(path: &std::path::Path) -> Option<Check> {
    let condition = store::probe(path)?;
    Some(
        Check::new("nmp.store", CheckStatus::Error, condition.summary())
            .target(
                condition
                    .path()
                    .map(str::to_string)
                    .unwrap_or_else(|| path.display().to_string()),
                condition.state(),
            )
            .repair(condition.remedy()),
    )
}

/// What the `daemon` check tells an operator once `nmp.store` has named the
/// real condition. Pointing at it beats repeating it, and beats today's
/// "restart the daemon" — a restart clears none of these.
fn daemon_repair(store_named_it: bool) -> &'static str {
    if store_named_it {
        "the `nmp.store` check names why this daemon exits at startup; act on that first — \
         restarting will not clear it"
    } else {
        "run `mosaico doctor --fix` for a session-preserving daemon restart"
    }
}

#[cfg(test)]
#[path = "store/tests.rs"]
mod tests;
