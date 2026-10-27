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
            match reload(&state).await {
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

async fn reload(state: &Arc<DaemonState>) -> Result<bool> {
    let _serial = state.config_reload.lock().await;
    let previous = state.config();
    let (next, next_backend_keys) = super::lifecycle::backend_identity::load_backend()?;
    if next == previous {
        return Ok(false);
    }

    if runtime_changed(&previous, &next) {
        reload_nostr_runtime(state, &next, next_backend_keys).await?;
        // Backoff earned against the previous relay must not delay the new one.
        if let Err(error) = state.with_store(|store| store.reset_profile_retirement_backoff()) {
            tracing::warn!(%error, "profile retirement backoff was not reset");
        }
    } else {
        let provider = provider_for(
            state.nostr(),
            state.nip29_view(),
            &next,
            &state.store,
            next_backend_keys.public_key(),
        );
        install_config(state, provider, next);
    }
    reconcile_managed_admins(state);
    Ok(true)
}

fn runtime_changed(previous: &Config, next: &Config) -> bool {
    previous.relay != next.relay || previous.backend_nsec() != next.backend_nsec()
}

async fn reload_nostr_runtime(
    state: &Arc<DaemonState>,
    config: &Config,
    backend_keys: Keys,
) -> Result<()> {
    let runtime = Arc::new(super::lifecycle::nostr_open::open(config)?);
    let view = Arc::new(crate::fabric::nip29::view::Nip29ViewStore::default());
    let provider = provider_for(
        runtime.clone(),
        view.clone(),
        config,
        &state.store,
        backend_keys.public_key(),
    );
    runtime
        .start()
        .await
        .context("starting replacement Nostr runtime")?;
    provider.spawn_view_rebuilder();

    super::group_records::shutdown(state).await;
    *state.subscriptions.reconciler.lock().unwrap() =
        crate::reconcile::SubscriptionReconciler::new();
    state.subscriptions.profile_authors.lock().unwrap().clear();
    let previous = state.nostr();
    install_runtime(state, runtime, view, provider, backend_keys, config.clone());
    previous.shutdown().await;
    super::spawn_demux(state.clone());
    super::subscriptions::sync_subscriptions(state).await?;
    Ok(())
}

fn provider_for(
    runtime: Arc<crate::nostr_runtime::NostrRuntime>,
    view: Arc<crate::fabric::nip29::view::Nip29ViewStore>,
    config: &Config,
    store: &Arc<Mutex<Store>>,
    management_pubkey: nostr::key::PublicKey,
) -> Arc<Nip29Provider> {
    Arc::new(Nip29Provider::new(
        runtime,
        view,
        store.clone(),
        management_pubkey,
        config.user_nsec().cloned(),
        config.whitelisted_pubkeys.clone(),
    ))
}

fn install_runtime(
    state: &DaemonState,
    runtime: Arc<crate::nostr_runtime::NostrRuntime>,
    view: Arc<crate::fabric::nip29::view::Nip29ViewStore>,
    provider: Arc<Nip29Provider>,
    backend_keys: Keys,
    config: Config,
) {
    state
        .store
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .bind_nip29_view(view.clone());
    *state
        .nostr
        .write()
        .unwrap_or_else(|poison| poison.into_inner()) = runtime;
    *state
        .nip29_view
        .write()
        .unwrap_or_else(|poison| poison.into_inner()) = view;
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

#[cfg(test)]
#[path = "config_reload/tests.rs"]
mod tests;
