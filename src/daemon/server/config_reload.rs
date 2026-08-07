//! Filesystem-driven replacement of the daemon's selected configuration.

use super::*;
use anyhow::{Context, Result};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

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
        install_config(state, provider_for(state.nmp(), &next, &state.store), next);
    }
    super::lifecycle::auth_restore::restore(state)
        .context("restoring identities after config reload")?;
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
    state.nmp().shutdown();
    match build_runtime(next, storage, next_backend_keys, &state.store) {
        Ok((nmp, provider)) => install_runtime(state, nmp, provider, next.clone()),
        Err(reload_error) => {
            let prior_keys = backend_keys(previous)?;
            let (nmp, provider) = build_runtime(previous, storage, &prior_keys, &state.store)
                .context("restoring the prior relay runtime")?;
            install_runtime(state, nmp, provider, previous.clone());
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

fn build_runtime(
    config: &Config,
    storage: &crate::daemon::storage_paths::StoragePaths,
    backend_keys: &Keys,
    store: &Arc<Mutex<Store>>,
) -> Result<(Arc<crate::nmp_host::NmpHost>, Arc<Nip29Provider>)> {
    let nmp = Arc::new(super::lifecycle::nmp_open::open(
        config,
        storage,
        backend_keys,
    )?);
    Ok((nmp.clone(), provider_for(nmp, config, store)))
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

fn backend_keys(config: &Config) -> Result<Keys> {
    let key = config
        .backend_nsec()
        .context("prior configuration has no mosaicoPrivateKey")?;
    Keys::parse(key).context("prior mosaicoPrivateKey is invalid")
}

fn install_runtime(
    state: &DaemonState,
    nmp: Arc<crate::nmp_host::NmpHost>,
    provider: Arc<Nip29Provider>,
    config: Config,
) {
    *state
        .nmp
        .write()
        .unwrap_or_else(|poison| poison.into_inner()) = nmp;
    *state
        .provider
        .write()
        .unwrap_or_else(|poison| poison.into_inner()) = provider;
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
