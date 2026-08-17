//! Filesystem-driven replacement of the daemon's selected configuration.

use super::*;
use anyhow::{Context, Result};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

#[path = "config_reload/admin_reconciliation.rs"]
mod admin_reconciliation;
pub(super) use admin_reconciliation::reconcile_managed_admins;
#[cfg(test)]
use admin_reconciliation::{managed_admin_targets, ManagedAdminTarget};

const SETTLE: Duration = Duration::from_millis(75);

pub(super) struct ConfigWatcher {
    _watcher: RecommendedWatcher,
}

/// Watch the containing directory rather than the file itself: setup and other
/// safe writers replace `config.json` atomically, which can invalidate a
/// file-only watch before the replacement arrives.
pub(super) fn watch(
    state: Arc<DaemonState>,
    storage: crate::daemon::storage_paths::StoragePaths,
) -> Result<ConfigWatcher> {
    let path = storage.config_path.clone();
    let parent = path
        .parent()
        .context("selected config path has no parent directory")?
        .to_path_buf();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut watcher =
        notify::recommended_watcher(move |result: notify::Result<notify::Event>| match result {
            // macOS may coalesce an atomic replacement into a directory event.
            // Parsing and comparing the selected config below makes unrelated
            // files in this small daemon-owned directory a harmless no-op.
            Ok(_) => {
                let _ = tx.send(());
            }
            Err(error) => tracing::warn!(%error, "config filesystem watch failed"),
        })
        .context("creating config filesystem watcher")?;
    watcher
        .watch(&parent, RecursiveMode::NonRecursive)
        .with_context(|| format!("watching {}", parent.display()))?;

    tokio::spawn(async move {
        while rx.recv().await.is_some() {
            tokio::time::sleep(SETTLE).await;
            while rx.try_recv().is_ok() {}
            match reload(&state, &storage) {
                Ok(true) => {
                    tracing::info!(config = %path.display(), "daemon configuration reloaded")
                }
                Ok(false) => {}
                Err(error) => tracing::warn!(
                    config = %path.display(),
                    error = %format!("{error:#}"),
                    "retaining the prior daemon configuration after reload failure"
                ),
            }
        }
    });

    Ok(ConfigWatcher { _watcher: watcher })
}

fn reload(
    state: &Arc<DaemonState>,
    storage: &crate::daemon::storage_paths::StoragePaths,
) -> Result<bool> {
    let _serial = state
        .config_reload
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let previous = state.snapshot();
    let (next, next_backend_keys) = super::lifecycle::auth_restore::load_backend()?;
    if next == previous.config {
        return Ok(false);
    }

    let next_nmp = if runtime_changed(&previous.config, &next) {
        Arc::new(super::lifecycle::nmp_open::open(
            &next,
            storage,
            &next_backend_keys,
        )?)
    } else {
        previous.nmp.clone()
    };
    let candidate = Arc::new(RuntimeSnapshot {
        generation: previous.generation + 1,
        provider: provider_for(next_nmp.clone(), &next, &state.store),
        nmp: next_nmp,
        config: next,
    });
    super::lifecycle::auth_restore::restore_for(state, &candidate)
        .context("restoring identities after config reload")?;
    let runtime_changed = !Arc::ptr_eq(&previous.nmp, &candidate.nmp);
    let retired = state.install_snapshot(candidate);
    if runtime_changed {
        super::group_records::shutdown(state);
        *state.subscriptions.reconciler.lock().unwrap() =
            crate::reconcile::SubscriptionReconciler::new();
        super::spawn_demux(state.clone());
        sync_subscriptions(state);
        retired.nmp.shutdown();
    }
    reconcile_managed_admins(state);
    Ok(true)
}

fn runtime_changed(previous: &Config, next: &Config) -> bool {
    previous.relays != next.relays
        || previous.indexer_relay != next.indexer_relay
        || previous.backend_nsec() != next.backend_nsec()
}

fn provider_for(
    nmp: Arc<crate::nmp_host::NmpHost>,
    config: &Config,
    store: &Arc<Mutex<Store>>,
) -> Arc<Nip29Provider> {
    Arc::new(Nip29Provider::new(
        nmp,
        store.clone(),
        config.management_nsec().cloned(),
        config.user_nsec().cloned(),
        config.whitelisted_pubkeys.clone(),
    ))
}

fn sync_subscriptions(state: &Arc<DaemonState>) {
    let state = state.clone();
    tokio::spawn(async move {
        if let Err(error) = super::subscriptions::sync_subscriptions(&state).await {
            tracing::warn!(error = %error, "config reload subscription sync failed");
        }
    });
}

#[cfg(test)]
#[path = "config_reload/tests.rs"]
mod tests;
