use super::*;
use crate::test_env::EnvGuard;

fn write(path: &std::path::Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

#[test]
fn conflict_selector_is_forwarded_for_daemon_side_realization() {
    let root = tempfile::tempdir().unwrap();
    let mosaico_home = root.path().join("mosaico");
    let codex_home = root.path().join(".codex");
    let mut env = EnvGuard::set("HOME", root.path());
    env.set_var("MOSAICO_HOME", &mosaico_home);
    env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
    env.set_var("CODEX_HOME", &codex_home);
    env.set_var("XDG_CONFIG_HOME", root.path().join(".config"));
    write(
        &codex_home.join("agents/writer.toml"),
        "name='writer'\ndescription='Writes'\ndeveloper_instructions='Write'",
    );
    write(
        &root.path().join(".claude/agents/writer.md"),
        "---\nname: writer\ndescription: Writes\n---\nWrite",
    );

    let catalog = crate::agent_catalog::AgentCatalog::discover(
        &crate::agent_catalog::DiscoveryRoots::installed().unwrap(),
        &[root.path().to_path_buf()],
    )
    .unwrap();
    let inventory = crate::agent_inventory::AgentInventory::build(
        &mosaico_home,
        &[
            crate::session::Harness::Codex,
            crate::session::Harness::ClaudeCode,
        ],
        &crate::harness::HarnessesConfig::default(),
        &catalog,
        Some(root.path()),
    );
    let selection = resolve_from_inventory("writer-codex", &inventory).unwrap();

    assert_eq!(selection.slug, "writer-codex");
    assert!(!mosaico_home.join("agents/writer.json").exists());
}
