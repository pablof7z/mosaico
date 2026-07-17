use super::*;
use crate::test_env::EnvGuard;

fn isolated_home() -> (EnvGuard, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let g = EnvGuard::set("MOSAICO_HOME", dir.path());
    (g, dir)
}

fn write_workspaces_map(entries: &[(&str, &str)]) {
    let mut obj = std::collections::BTreeMap::new();
    for (slug, path) in entries {
        obj.insert(slug.to_string(), path.to_string());
    }
    write_map(&obj).unwrap();
}

// -- resolve: git path ----------------------------------------------------

// No git tests here: git resolution depends on the host having git and
// the tempdir not being inside a repo. Covered by integration tests.

// -- resolve: workspaces.json path ----------------------------------------

#[test]
fn resolve_returns_slug_when_cwd_is_in_map() {
    let (_g, _home) = isolated_home();
    let dir = tempfile::tempdir().unwrap();
    let abs = std::fs::canonicalize(dir.path()).unwrap();
    write_workspaces_map(&[("my-workspace", abs.to_str().unwrap())]);
    assert_eq!(resolve(&abs).unwrap(), "my-workspace");
}

#[test]
fn resolve_walks_upward_to_find_ancestor_in_map() {
    let (_g, _home) = isolated_home();
    let dir = tempfile::tempdir().unwrap();
    let abs = std::fs::canonicalize(dir.path()).unwrap();
    write_workspaces_map(&[("ancestor-ws", abs.to_str().unwrap())]);
    let nested = abs.join("a").join("b");
    std::fs::create_dir_all(&nested).unwrap();
    assert_eq!(resolve(&nested).unwrap(), "ancestor-ws");
}

#[test]
fn resolve_errors_when_no_git_and_not_in_map() {
    let (_g, _home) = isolated_home();
    let dir = tempfile::tempdir().unwrap();
    let abs = std::fs::canonicalize(dir.path()).unwrap();
    // Empty map, no git (tempdir is outside any repo on macOS).
    write_workspaces_map(&[]);
    let result = resolve(&abs);
    // tempdir is outside any repo, so this should be Err.
    // (If the tempdir happens to be inside a git repo on CI, skip.)
    if git_toplevel(&abs).is_none() {
        assert!(result.is_err());
    }
}

#[test]
fn resolve_rejects_a_corrupt_workspace_map() {
    let (_g, home) = isolated_home();
    let dir = tempfile::tempdir().unwrap();
    let abs = std::fs::canonicalize(dir.path()).unwrap();
    std::fs::write(home.path().join("workspaces.json"), "{not json").unwrap();

    if git_toplevel(&abs).is_none() {
        let error = resolve(&abs).unwrap_err();
        assert!(
            format!("{error:#}").contains("corrupt workspace map"),
            "error = {error:#}"
        );
    }
}

// -- workspace_dir ---------------------------------------------------------

#[test]
fn workspace_dir_from_map_returns_ancestor_dir() {
    let (_g, _home) = isolated_home();
    let dir = tempfile::tempdir().unwrap();
    let abs = std::fs::canonicalize(dir.path()).unwrap();
    write_workspaces_map(&[("ws", abs.to_str().unwrap())]);
    let nested = abs.join("sub");
    std::fs::create_dir_all(&nested).unwrap();
    assert_eq!(workspace_dir(&nested).unwrap(), Some(abs));
}

// -- rel_cwd --------------------------------------------------------------

#[test]
fn rel_cwd_root_is_dot() {
    let (_g, _home) = isolated_home();
    let dir = tempfile::tempdir().unwrap();
    let abs = std::fs::canonicalize(dir.path()).unwrap();
    write_workspaces_map(&[("p", abs.to_str().unwrap())]);
    assert_eq!(rel_cwd(&abs).unwrap(), ".");
}

#[test]
fn rel_cwd_subdir_is_relative_joined() {
    let (_g, _home) = isolated_home();
    let dir = tempfile::tempdir().unwrap();
    let abs = std::fs::canonicalize(dir.path()).unwrap();
    write_workspaces_map(&[("p", abs.to_str().unwrap())]);
    let sub = abs.join("worktree1").join("nested");
    std::fs::create_dir_all(&sub).unwrap();
    assert_eq!(rel_cwd(&sub).unwrap(), "worktree1/nested");
}

// -- register_workspace ---------------------------------------------------

#[test]
fn register_workspace_writes_basename_and_path() {
    let (_g, _home) = isolated_home();
    let dir = tempfile::tempdir().unwrap();
    let abs = std::fs::canonicalize(dir.path()).unwrap();
    // Create a subdir with a known name so basename is deterministic.
    let ws_dir = abs.join("the-ws");
    std::fs::create_dir_all(&ws_dir).unwrap();
    let (slug, written_path) = register_workspace(&ws_dir, false).unwrap();
    assert_eq!(slug, "the-ws");
    assert_eq!(written_path, std::fs::canonicalize(&ws_dir).unwrap());
    // The map now contains the entry.
    let map = read_map().unwrap();
    assert_eq!(
        map.get("the-ws").map(|s| s.as_str()),
        Some(std::fs::canonicalize(&ws_dir).unwrap().to_str().unwrap())
    );
}

#[test]
fn register_workspace_errors_on_duplicate_slug_different_path() {
    let (_g, _home) = isolated_home();
    let a = tempfile::tempdir().unwrap();
    let a_abs = std::fs::canonicalize(a.path()).unwrap();
    let a_ws = a_abs.join("dup");
    std::fs::create_dir_all(&a_ws).unwrap();
    register_workspace(&a_ws, false).unwrap();

    // Different path, same slug basename.
    let b = tempfile::tempdir().unwrap();
    let b_abs = std::fs::canonicalize(b.path()).unwrap();
    let b_ws = b_abs.join("dup");
    std::fs::create_dir_all(&b_ws).unwrap();
    let err = register_workspace(&b_ws, false);
    assert!(err.is_err());
    let msg = format!("{}", err.unwrap_err());
    assert!(
        msg.contains("already mapped") || msg.contains("already in use"),
        "msg = {msg}"
    );
}

#[test]
fn register_workspace_force_overwrites_duplicate() {
    let (_g, _home) = isolated_home();
    let a = tempfile::tempdir().unwrap();
    let a_abs = std::fs::canonicalize(a.path()).unwrap();
    let a_ws = a_abs.join("dup");
    std::fs::create_dir_all(&a_ws).unwrap();
    register_workspace(&a_ws, false).unwrap();

    let b = tempfile::tempdir().unwrap();
    let b_abs = std::fs::canonicalize(b.path()).unwrap();
    let b_ws = b_abs.join("dup");
    std::fs::create_dir_all(&b_ws).unwrap();
    let (slug, path) = register_workspace(&b_ws, true).unwrap();
    assert_eq!(slug, "dup");
    assert_eq!(path, std::fs::canonicalize(&b_ws).unwrap());
}

#[test]
fn register_workspace_idempotent_when_same_path() {
    let (_g, _home) = isolated_home();
    let dir = tempfile::tempdir().unwrap();
    let abs = std::fs::canonicalize(dir.path()).unwrap();
    let ws = abs.join("idem");
    std::fs::create_dir_all(&ws).unwrap();
    register_workspace(&ws, false).unwrap();
    // Registering again with the same path should succeed (no-op).
    let (slug, path) = register_workspace(&ws, false).unwrap();
    assert_eq!(slug, "idem");
    assert_eq!(path, std::fs::canonicalize(&ws).unwrap());
}
