use super::*;
use crate::test_env::EnvGuard;

fn write(path: &std::path::Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

#[test]
fn only_absent_capabilities_are_tombstoned() {
    let advertised = vec![CapabilityAdvertisement {
        slug: "reviewer".into(),
        use_criteria: "Reviews changes".into(),
        root_channels: Vec::new(),
        available_since: 1,
    }];

    assert!(!should_tombstone(&advertised, "reviewer"));
    assert!(should_tombstone(&advertised, "deleted"));
    assert!(!should_tombstone(&advertised, ""));
}

#[tokio::test]
async fn discovered_capabilities_are_global_or_workspace_scoped() {
    let home = tempfile::tempdir().unwrap();
    let mosaico_home = home.path().join("mosaico");
    let codex_home = home.path().join(".codex");
    let mut env = EnvGuard::set("MOSAICO_HOME", &mosaico_home);
    env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
    env.set_var("HOME", home.path());
    env.set_var("CODEX_HOME", &codex_home);
    write(
        &mosaico_home.join("harnesses.json"),
        r#"{"codex-rpc":{"harness":"codex","transport":"app-server"}}"#,
    );
    write(
        &codex_home.join("agents/global.toml"),
        "name='global'\ndescription='Everywhere'\ndeveloper_instructions='Work'",
    );
    let work_a = home.path().join("work-a");
    let work_b = home.path().join("work-b");
    std::fs::create_dir_all(&work_b).unwrap();
    write(
        &work_a.join(".codex/agents/project.toml"),
        "name='project'\ndescription='Only A'\ndeveloper_instructions='Work'",
    );
    let state = DaemonState::new_for_test().await;
    state.with_store(|store| {
        store.upsert_channel("root-a", "root-a", "", "", 1).unwrap();
        store.upsert_channel("root-b", "root-b", "", "", 1).unwrap();
        store
            .upsert_workspace("root-a", &work_a.to_string_lossy(), 1)
            .unwrap();
        store
            .upsert_workspace("root-b", &work_b.to_string_lossy(), 1)
            .unwrap();
    });
    state.refresh_agent_catalog().unwrap();

    let (advertised, failed) = capability_advertisements(&state).unwrap();
    assert!(failed.is_empty(), "{failed:?}");
    let global = advertised
        .iter()
        .find(|agent| agent.slug == "global")
        .unwrap();
    assert_eq!(global.root_channels, ["root-a", "root-b"]);
    let project = advertised
        .iter()
        .find(|agent| agent.slug == "project")
        .unwrap();
    assert_eq!(project.root_channels, ["root-a"]);
}
