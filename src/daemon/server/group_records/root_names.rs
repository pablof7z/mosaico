//! Repairing a workspace root whose relay-signed name drifted off its group id.
//!
//! Driven by the retained group-records observation and by nothing else. The
//! roots this backend may rename are exactly the ones whose relay-signed admin
//! list names the management key — the question the observation stands open on
//! — so there is no relay enumeration to filter down afterwards, and no bound
//! or timeout deciding how much of the answer arrives.
//!
//! A root is considered only when THIS delivery carried a record for it. The
//! name being compared is then relay truth: `relay_channels` also holds the
//! local row `channel_init` writes for a workspace root before the group is
//! provisioned, and a reservation is not something to publish a rename against.

use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::Result;

use super::DaemonState;
use crate::state::Store;

/// Rename every root in `delivered` whose relay-signed name is not its group id.
///
/// `attempted` bounds this to one publish per root for the life of the
/// observation. An accepted rename needs no retry — the relay's next record
/// carries the corrected name and stops matching — and a rejected one must not
/// be re-published on every subsequent delivery.
///
/// The publishes are spawned rather than awaited: this runs on the drain that
/// folds deliveries into the cache, and a slow relay write must not hold up the
/// next snapshot.
pub(super) fn repair_delivered(
    state: &Arc<DaemonState>,
    delivered: &BTreeSet<String>,
    attempted: &mut BTreeSet<String>,
) {
    let Some(backend_pubkey) = state.backend_pubkey() else {
        return;
    };
    let candidates: BTreeSet<String> = delivered.difference(attempted).cloned().collect();
    if candidates.is_empty() {
        return;
    }
    let roots = match state
        .with_store(|store| roots_needing_workspace_name(store, &backend_pubkey, &candidates))
    {
        Ok(roots) => roots,
        Err(error) => {
            tracing::error!(%error, "workspace root repair authority lookup failed");
            return;
        }
    };
    if roots.is_empty() {
        return;
    }
    attempted.extend(roots.iter().cloned());
    let state = state.clone();
    tokio::spawn(async move {
        for root in roots {
            if !state.provider().nip29_set_group_name(&root, &root).await {
                tracing::warn!(
                    channel = %root,
                    "workspace root name repair was rejected"
                );
            }
        }
    });
}

/// The delivered roots this backend both manages and must rename.
///
/// "Manages" is either the relay-signed admin grant or a local workspace
/// binding: a root bound to a directory on this machine is this backend's to
/// keep named even while the admin record is in flight.
fn roots_needing_workspace_name(
    store: &Store,
    backend_pubkey: &str,
    delivered: &BTreeSet<String>,
) -> Result<Vec<String>> {
    let mut roots = Vec::new();
    for channel in store.list_root_channels()? {
        if !delivered.contains(&channel.channel_h) {
            continue;
        }
        if channel.name.trim() == channel.channel_h {
            continue;
        }
        if store.is_channel_admin(&channel.channel_h, backend_pubkey)?
            || crate::daemon::workspace_path::WorkspacePathResolver::new(store)
                .path_for_channel(&channel.channel_h)?
                .is_some()
        {
            roots.push(channel.channel_h);
        }
    }
    Ok(roots)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded() -> Store {
        let store = Store::open_memory().unwrap();
        store.upsert_channel("one", "wrong", "", "", 1).unwrap();
        store.upsert_channel("two", "two", "", "", 1).unwrap();
        store.upsert_channel("remote", "remote", "", "", 1).unwrap();
        store
            .upsert_channel("bound", "also-wrong", "", "", 1)
            .unwrap();
        store.upsert_workspace("bound", "/work/bound", 1).unwrap();
        store
            .upsert_channel_member("one", "backend", "admin", 1)
            .unwrap();
        store
            .upsert_channel_member("two", "backend", "admin", 1)
            .unwrap();
        store
    }

    fn delivery(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn only_managed_misnamed_roots_need_repair() {
        let store = seeded();
        assert_eq!(
            roots_needing_workspace_name(
                &store,
                "backend",
                &delivery(&["one", "two", "remote", "bound"])
            )
            .unwrap(),
            vec!["bound", "one"]
        );
    }

    #[test]
    fn a_root_this_delivery_did_not_carry_is_left_alone() {
        let store = seeded();
        assert_eq!(
            roots_needing_workspace_name(&store, "backend", &delivery(&["one"])).unwrap(),
            vec!["one"]
        );
        assert!(
            roots_needing_workspace_name(&store, "backend", &delivery(&["two"]))
                .unwrap()
                .is_empty(),
            "a correctly named root needs no repair"
        );
        assert!(
            roots_needing_workspace_name(&store, "backend", &BTreeSet::new())
                .unwrap()
                .is_empty(),
            "an empty delivery repairs nothing, however stale the cache is"
        );
    }
}
