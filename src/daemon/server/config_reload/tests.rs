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

#[tokio::test]
async fn managed_child_inherits_the_parents_desired_generation_without_roster_polling() {
    let configured = Keys::generate().public_key().to_hex();
    let removed = Keys::generate().public_key().to_hex();
    let state = DaemonState::new_for_test_with_whitelisted(vec![configured.clone()]).await;
    let management = state.provider().management_pubkey().unwrap();
    state
        .with_store(|store| {
            store.install_test_nmp_group_delivery(crate::state::TestGroupDelivery::new([
                crate::state::TestGroup::new("root")
                    .metadata("Root", "", "", 1)
                    .admins(vec![
                        management.clone(),
                        configured.clone(),
                        removed.clone(),
                    ]),
                crate::state::TestGroup::new("child")
                    .metadata("Child", "", "root", 2)
                    .admins(vec![management.clone(), configured.clone(), removed]),
            ]));
            store.upsert_workspace("root", "/tmp/root", 1)?;
            store.reserve_channel_resolution_intent("root", "child", "child", 2)?;
            Ok::<_, anyhow::Error>(())
        })
        .unwrap();

    let targets = managed_admin_targets(&state, &state.provider()).unwrap();
    assert_eq!(
        targets,
        vec![
            ManagedAdminTarget {
                channel: "root".into(),
                managed_parent: None,
                inherited_admins: Vec::new(),
            },
            ManagedAdminTarget {
                channel: "child".into(),
                managed_parent: Some("root".into()),
                inherited_admins: vec![management, configured],
            },
        ]
    );
}

#[tokio::test]
async fn managed_child_of_an_external_group_inherits_the_observed_parent_admins() {
    let inherited = Keys::generate().public_key().to_hex();
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|store| {
            store.install_test_nmp_group_delivery(crate::state::TestGroupDelivery::new([
                crate::state::TestGroup::new("external")
                    .metadata("External", "", "", 1)
                    .admins(vec![inherited.clone()]),
                crate::state::TestGroup::new("child").metadata("Child", "", "external", 2),
            ]));
            store.reserve_channel_resolution_intent("external", "child", "child", 2)?;
            Ok::<_, anyhow::Error>(())
        })
        .unwrap();

    assert_eq!(
        managed_admin_targets(&state, &state.provider()).unwrap(),
        vec![ManagedAdminTarget {
            channel: "child".into(),
            managed_parent: None,
            inherited_admins: vec![inherited],
        }]
    );
}
