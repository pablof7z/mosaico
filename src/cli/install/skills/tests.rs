use super::*;
use crate::test_env::EnvGuard;

fn write_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn opts(dry_run: bool, uninstall: bool) -> InstallOpts {
    InstallOpts {
        dry_run,
        uninstall,
        ..InstallOpts::default()
    }
}

#[test]
fn install_writes_complete_bundled_agents_skill() {
    let temp = tempfile::tempdir().unwrap();
    let _home = EnvGuard::set("HOME", temp.path());

    install(&opts(false, false)).unwrap();

    let installed = temp.path().join(".agents/skills/mosaico");
    assert!(installed.is_dir());
    assert!(!installed.is_symlink());
    assert!(is_bundled_skill(&installed));
    assert_eq!(
        std::fs::read_to_string(installed.join("agents/openai.yaml")).unwrap(),
        include_str!("../../../../skills/mosaico/agents/openai.yaml")
    );
}

#[test]
fn install_links_claude_to_owned_agents_skill_when_detected() {
    let temp = tempfile::tempdir().unwrap();
    write_executable(&temp.path().join(".local/bin/claude"));
    let _home = EnvGuard::set("HOME", temp.path());

    install(&opts(false, false)).unwrap();

    let agents = temp.path().join(".agents/skills/mosaico");
    let link = temp.path().join(".claude/skills/mosaico");
    assert!(link.is_symlink());
    assert_eq!(link.canonicalize().unwrap(), agents.canonicalize().unwrap());
    assert!(is_bundled_skill(&agents));
}

#[test]
fn uninstall_removes_owned_copy_and_harness_link() {
    let temp = tempfile::tempdir().unwrap();
    write_executable(&temp.path().join(".local/bin/claude"));
    let _home = EnvGuard::set("HOME", temp.path());

    install(&opts(false, false)).unwrap();
    install(&opts(false, true)).unwrap();

    assert!(!temp.path().join(".agents/skills/mosaico").exists());
    assert!(temp
        .path()
        .join(".claude/skills/mosaico")
        .symlink_metadata()
        .is_err());
}

#[test]
fn install_replaces_checkout_symlink_with_owned_copy() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("checkout/skills/mosaico");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("SKILL.md"), "name: mosaico\nold").unwrap();

    let installed = temp.path().join(".agents/skills/mosaico");
    std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&source, &installed).unwrap();

    let _home = EnvGuard::set("HOME", temp.path());
    install(&opts(false, false)).unwrap();

    assert!(installed.is_dir());
    assert!(!installed.is_symlink());
    assert!(is_bundled_skill(&installed));
    assert!(source.join("SKILL.md").is_file());
}

#[test]
fn install_replaces_stale_copied_tree_without_leaving_files() {
    let temp = tempfile::tempdir().unwrap();
    let installed = temp.path().join(".agents/skills/mosaico");
    std::fs::create_dir_all(&installed).unwrap();
    std::fs::write(installed.join("STALE.md"), "removed upstream").unwrap();

    let _home = EnvGuard::set("HOME", temp.path());
    install(&opts(false, false)).unwrap();

    assert!(is_bundled_skill(&installed));
    assert!(!installed.join("STALE.md").exists());
}

#[test]
fn health_classifies_missing_stale_and_healthy_owned_copy() {
    let temp = tempfile::tempdir().unwrap();
    let installed = temp.path().join(".agents/skills/mosaico");
    let _home = EnvGuard::set("HOME", temp.path());

    let missing = health().unwrap();
    assert_eq!(missing.canonical_path, installed);
    assert_eq!(missing.targets[0].state, SkillHealthState::Missing);

    std::fs::create_dir_all(&installed).unwrap();
    std::fs::write(installed.join("SKILL.md"), "name: mosaico\nstale").unwrap();
    assert_eq!(health().unwrap().targets[0].state, SkillHealthState::Stale);

    let repaired = repair().unwrap();
    assert_eq!(repaired.canonical_path, installed);
    assert_eq!(repaired.targets[0].state, SkillHealthState::Healthy);
    assert!(!repaired.canonical_path.is_symlink());
}

#[test]
fn health_reports_detected_claude_target_separately() {
    let temp = tempfile::tempdir().unwrap();
    write_executable(&temp.path().join(".local/bin/claude"));
    let _home = EnvGuard::set("HOME", temp.path());

    let missing = health().unwrap();
    assert_eq!(missing.targets.len(), 2);
    assert_eq!(missing.targets[0].label, "agents");
    assert_eq!(missing.targets[0].state, SkillHealthState::Missing);
    assert_eq!(missing.targets[1].label, "claude");
    assert_eq!(missing.targets[1].state, SkillHealthState::Missing);

    repair().unwrap();
    std::fs::remove_file(temp.path().join(".claude/skills/mosaico")).unwrap();
    let partial = health().unwrap();
    assert_eq!(partial.targets[0].state, SkillHealthState::Healthy);
    assert_eq!(partial.targets[1].state, SkillHealthState::Missing);
}
