use super::*;
use crate::state::{TestGroup, TestGroupDelivery};
use crate::test_env::EnvGuard;

fn write(path: &std::path::Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn write_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;

    write(path, "#!/bin/sh\n");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[tokio::test]
async fn snapshot_owns_agents_and_workspaces_independently() {
    let home = tempfile::tempdir().unwrap();
    let mosaico_home = home.path().join("mosaico");
    let codex_home = home.path().join(".codex");
    let mut env = EnvGuard::set("MOSAICO_HOME", &mosaico_home);
    env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
    env.set_var("HOME", home.path());
    env.set_var("CODEX_HOME", &codex_home);
    env.set_var("PATH", home.path().join(".local/bin"));
    write(
        &codex_home.join("agents/global.toml"),
        "name='global'\ndescription='Everywhere'\ndeveloper_instructions='Work'",
    );
    write_executable(&home.path().join(".local/bin/codex"));
    let work_a = home.path().join("work-a");
    let work_b = home.path().join("work-b");
    std::fs::create_dir_all(&work_b).unwrap();
    write(
        &work_a.join(".codex/agents/project.toml"),
        "name='project'\ndescription='Project work'\ndeveloper_instructions='Work'",
    );
    let state = DaemonState::new_for_test().await;
    let management = state.backend_pubkey().unwrap();
    state.with_store(|store| {
        // Observed but not administered: cached because the relay describes it,
        // never this backend's to advertise.
        store.install_test_nmp_group_delivery(TestGroupDelivery::new([
            TestGroup::new("root-a")
                .metadata("root-a", "", "", 1)
                .admins(vec![management.clone()]),
            TestGroup::new("root-b")
                .metadata("root-b", "", "", 1)
                .admins(vec![management.clone()]),
            TestGroup::new("someone-elses").metadata("someone-elses", "", "", 1),
        ]));
        store
            .upsert_workspace("root-a", &work_a.to_string_lossy(), 1)
            .unwrap();
        store
            .upsert_workspace("root-b", &work_b.to_string_lossy(), 1)
            .unwrap();
    });
    state.refresh_agent_catalog().unwrap();
    *state.catalog.harnesses.lock().unwrap() = vec![crate::session::Harness::Codex];

    let snapshot = backend_profile_snapshot(&state).unwrap();

    assert!(snapshot.failures.is_empty(), "{:?}", snapshot.failures);
    assert_eq!(snapshot.workspaces, ["root-a", "root-b"]);
    assert_eq!(
        snapshot
            .agents
            .iter()
            .map(|agent| (agent.slug.as_str(), agent.about.as_str()))
            .collect::<Vec<_>>(),
        [
            ("codex", ""),
            ("global", "Everywhere"),
            ("project", "Project work"),
        ]
    );
}
