use super::*;
use crate::config::{BoundaryAction, CrossProjectBoundary};

fn document(key: &str, relay: &str, read: &str, write: &str) -> String {
    serde_json::json!({
        "relays": [relay],
        "mosaicoPrivateKey": key,
        "agents": { "behavior": { "crossProjectBoundary": { "read": read, "write": write } } },
    })
    .to_string()
}

fn replace(path: &std::path::Path, content: &str) {
    let staged = path.with_extension("next");
    std::fs::write(&staged, content).unwrap();
    std::fs::rename(staged, path).unwrap();
}

async fn wait_for(state: &Arc<DaemonState>, expected: CrossProjectBoundary) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if state.config().cross_project_boundary == expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("filesystem event should reload the selected configuration");
}

async fn wait_for_relay_runtime(
    state: &Arc<DaemonState>,
    previous: &Arc<crate::nmp_host::NmpHost>,
) -> Arc<crate::nmp_host::NmpHost> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let current = state.nmp();
            if !Arc::ptr_eq(previous, &current) {
                return current;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("relay change should replace the NMP runtime")
}

#[tokio::test]
async fn atomic_config_replacement_updates_runtime_without_daemon_restart() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("mosaico");
    std::fs::create_dir_all(&home).unwrap();
    let path = home.join("config.json");
    let key = Keys::generate().secret_key().to_secret_hex();
    std::fs::write(&path, document(&key, "wss://relay.example", "warn", "deny")).unwrap();
    let mut env = crate::test_env::EnvGuard::set("MOSAICO_HOME", &home);
    env.set_var("MOSAICO_CONFIG", &path);
    let state = DaemonState::new_for_test().await;
    let initial = Config::load().unwrap();
    install_config(
        &state,
        provider_for(state.nmp(), &initial, &state.store),
        initial,
    );
    let daemon_before = state.nmp();
    let _watcher = watch(
        state.clone(),
        crate::daemon::storage_paths::StoragePaths::current(),
    )
    .unwrap();

    replace(
        &path,
        &document(&key, "wss://relay.example", "allow", "allow"),
    );
    wait_for(
        &state,
        CrossProjectBoundary {
            read: BoundaryAction::Allow,
            write: BoundaryAction::Allow,
        },
    )
    .await;
    let daemon_after = state.nmp();
    assert!(Arc::ptr_eq(&daemon_before, &daemon_after));

    let replacement_key = Keys::generate().secret_key().to_secret_hex();
    replace(
        &path,
        &document(&replacement_key, "wss://relay.example", "allow", "allow"),
    );
    let daemon_after = wait_for_relay_runtime(&state, &daemon_after).await;

    replace(
        &path,
        &document(
            &replacement_key,
            "wss://relay-two.example",
            "allow",
            "allow",
        ),
    );
    let daemon_after = wait_for_relay_runtime(&state, &daemon_after).await;

    replace(&path, "{");
    tokio::time::sleep(SETTLE * 2).await;
    assert!(Arc::ptr_eq(&daemon_after, &state.nmp()));
    assert_eq!(
        state.config().cross_project_boundary,
        CrossProjectBoundary {
            read: BoundaryAction::Allow,
            write: BoundaryAction::Allow,
        }
    );
}
