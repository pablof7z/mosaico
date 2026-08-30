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
    let previous = state.config();
    let (next, next_backend_keys) = super::lifecycle::auth_restore::load_backend()?;
    if next == previous {
        return Ok(false);
    }

    if runtime_changed(&previous, &next) {
        reload_relay_runtime(state, storage, &previous, &next, &next_backend_keys)?;
    } else {
        install_config(state, provider_for(state.fava(), &next, &state.store), next);
    }
    super::lifecycle::auth_restore::restore(state)
        .context("restoring identities after config reload")?;
    reconcile_managed_admins(state);
    Ok(true)
}

fn runtime_changed(previous: &Config, next: &Config) -> bool {
    previous.relays != next.relays
        || previous.indexer_relay != next.indexer_relay
        || previous.backend_nsec() != next.backend_nsec()
}

fn reload_relay_runtime(
    state: &Arc<DaemonState>,
    storage: &crate::daemon::storage_paths::StoragePaths,
    previous: &Config,
    next: &Config,
    next_backend_keys: &Keys,
) -> Result<()> {
    super::group_records::shutdown(state);
    *state.subscriptions.reconciler.lock().unwrap() =
        crate::reconcile::SubscriptionReconciler::new();
    state.fava().shutdown();
    // The fava write store (redb) holds an exclusive file lock for the
    // lifetime of the FavaHost. Drop the old host before opening the
    // replacement so the new build can acquire the lock.
    release_store_lock(state, next_backend_keys)?;
    match build_runtime(next, storage, next_backend_keys, &state.store) {
        Ok((fava, provider)) => install_runtime(
            state,
            fava,
            provider,
            next_backend_keys.clone(),
            next.clone(),
        ),
        Err(reload_error) => {
            let prior_keys = backend_keys(previous)?;
            let (fava, provider) = build_runtime(previous, storage, &prior_keys, &state.store)
                .context("restoring the prior relay runtime")?;
            install_runtime(state, fava, provider, prior_keys, previous.clone());
            super::lifecycle::auth_restore::restore(state)
                .context("restoring identities after failed config reload")?;
            super::spawn_demux(state.clone());
            sync_subscriptions(state);
            return Err(reload_error);
        }
    }
    super::spawn_demux(state.clone());
    sync_subscriptions(state);
    Ok(())
}

/// Swap the current FavaHost for a lock-free in-memory placeholder so the
/// redb exclusive lock is released before `build_runtime` reopens the file.
fn release_store_lock(state: &Arc<DaemonState>, keys: &Keys) -> Result<()> {
    let placeholder = Arc::new(
        crate::fava_host::FavaHost::open(&[], None, None, keys)
            .context("creating lock-release placeholder FavaHost")?,
    );
    *state.fava.write().unwrap_or_else(|p| p.into_inner()) = placeholder;
    Ok(())
}

fn build_runtime(
    config: &Config,
    storage: &crate::daemon::storage_paths::StoragePaths,
    backend_keys: &Keys,
    store: &Arc<Mutex<Store>>,
) -> Result<(Arc<crate::fava_host::FavaHost>, Arc<Nip29Provider>)> {
    let fava = Arc::new(super::lifecycle::fava_open::open(
        config,
        storage,
        backend_keys,
    )?);
    Ok((fava.clone(), provider_for(fava, config, store)))
}

fn provider_for(
    fava: Arc<crate::fava_host::FavaHost>,
    config: &Config,
    store: &Arc<Mutex<Store>>,
) -> Arc<Nip29Provider> {
    Arc::new(Nip29Provider::new(
        fava,
        store.clone(),
        config.user_nsec().cloned(),
        config.whitelisted_pubkeys.clone(),
    ))
}

fn backend_keys(config: &Config) -> Result<Keys> {
    let key = config
        .backend_nsec()
        .context("prior configuration has no mosaicoPrivateKey")?;
    Keys::parse(key).context("prior mosaicoPrivateKey is invalid")
}

fn install_runtime(
    state: &DaemonState,
    fava: Arc<crate::fava_host::FavaHost>,
    provider: Arc<Nip29Provider>,
    backend_keys: Keys,
    config: Config,
) {
    state
        .store
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .bind_observed_state(fava.views_handle(), fava.clone());
    *state
        .fava
        .write()
        .unwrap_or_else(|poison| poison.into_inner()) = fava;
    *state
        .provider
        .write()
        .unwrap_or_else(|poison| poison.into_inner()) = provider;
    *state
        .backend_keys
        .write()
        .unwrap_or_else(|poison| poison.into_inner()) = backend_keys;
    *state
        .cfg
        .write()
        .unwrap_or_else(|poison| poison.into_inner()) = config;
}

fn install_config(state: &DaemonState, provider: Arc<Nip29Provider>, config: Config) {
    *state
        .provider
        .write()
        .unwrap_or_else(|poison| poison.into_inner()) = provider;
    *state
        .cfg
        .write()
        .unwrap_or_else(|poison| poison.into_inner()) = config;
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
