use super::*;
use crate::test_env::EnvGuard;
use std::os::unix::fs::PermissionsExt as _;

fn detected_codex_home() -> (tempfile::TempDir, EnvGuard) {
    let home = tempfile::tempdir().unwrap();
    let bin = home.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let codex = bin.join("codex");
    std::fs::write(&codex, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut env = EnvGuard::set("HOME", home.path());
    env.set_var("PATH", &bin);
    (home, env)
}

#[test]
fn detected_but_unselected_harness_is_not_modified() {
    let (home, _env) = detected_codex_home();
    let path = home.path().join(".codex/hooks.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let foreign = r#"{"hooks":{"Stop":[{"hooks":[{"command":"foreign"}]}]}}"#;
    std::fs::write(&path, foreign).unwrap();
    let mut actions = Vec::new();

    repair_selected_integrations(&mut actions).unwrap();

    assert!(actions.is_empty());
    assert_eq!(std::fs::read_to_string(path).unwrap(), foreign);
}

#[test]
fn selected_stale_harness_is_repaired_and_reported() {
    let (home, _env) = detected_codex_home();
    let path = home.path().join(".codex/hooks.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{"hooks":{"Stop":[{"hooks":[{"command":"mosaico harness hook codex --type stop"}]}]}}"#,
    )
    .unwrap();
    let mut actions = Vec::new();

    repair_selected_integrations(&mut actions).unwrap();

    assert_eq!(actions.len(), 1);
    assert!(actions[0].contains("repaired Codex integration"));
    let harness = super::super::super::install::harnesses()
        .unwrap()
        .into_iter()
        .find(|harness| harness.id == "codex")
        .unwrap();
    assert!(super::super::super::install::is_installed(&harness));
}
