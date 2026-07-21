use super::*;

fn install_plugin() {
    for (path, body) in plugin_files().unwrap() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
}

fn install_fake_goose(home: &std::path::Path, version: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt as _;

    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let path = bin.join("goose");
    std::fs::write(&path, format!("#!/bin/sh\necho {version}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

#[test]
fn launch_requires_enabled_plugin_and_binds_unique_file() {
    let home = tempfile::tempdir().unwrap();
    let mosaico = home.path().join("mosaico");
    let mut env = crate::test_env::EnvGuard::set("HOME", home.path());
    env.set_var("MOSAICO_HOME", &mosaico);
    env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
    env.set_var("XDG_CONFIG_HOME", home.path().join(".config"));
    env.set_var("PATH", install_fake_goose(home.path(), "1.43.0"));
    assert!(prepare_launch_env(&mut Vec::new(), "goose-1").is_err());

    install_plugin();
    let mut launch_env = Vec::new();
    prepare_launch_env(&mut launch_env, "goose-1").unwrap();
    assert_eq!(launch_env.len(), 1);
    assert!(launch_env[0]
        .1
        .ends_with("harness-context/goose/goose-1.md"));

    std::fs::create_dir_all(home.path().join(".config/goose")).unwrap();
    std::fs::write(
        home.path().join(".config/goose/settings.json"),
        r#"{"disabledPlugins":["mosaico"]}"#,
    )
    .unwrap();
    assert!(prepare_launch_env(&mut Vec::new(), "goose-2").is_err());
    enable_plugin().unwrap();
    prepare_launch_env(&mut Vec::new(), "goose-2").unwrap();

    let plugin = plugin_root().unwrap().to_string_lossy().into_owned();
    std::fs::write(
        home.path().join(".config/goose/config.yaml"),
        serde_json::json!({
            "extensions": {"tom": {"enabled": false}},
            "plugins": {plugin: {"enabled": false}}
        })
        .to_string(),
    )
    .unwrap();
    assert!(prepare_launch_env(&mut Vec::new(), "goose-3").is_err());
    enable_plugin().unwrap();
    prepare_launch_env(&mut Vec::new(), "goose-3").unwrap();
}

#[test]
fn hook_retains_baseline_and_prepends_nonempty_deltas() {
    let home = tempfile::tempdir().unwrap();
    let mosaico = home.path().join("mosaico");
    let mut env = crate::test_env::EnvGuard::set("MOSAICO_HOME", &mosaico);
    env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
    let root = context_root().unwrap();
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("session.md");
    env.set_var(MOIM_ENV, &path);

    sync_hook_context("user-prompt-submit", Some("snapshot")).unwrap();
    sync_hook_context("post-tool-use", None).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "snapshot");

    sync_hook_context("post-tool-use", Some("delta")).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "delta\n\nsnapshot");

    sync_hook_context("user-prompt-submit", None).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "delta\n\nsnapshot");

    sync_hook_context("user-prompt-submit", Some("next turn")).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "next turn\n\ndelta\n\nsnapshot"
    );
}

#[test]
fn declared_small_context_window_is_rejected() {
    let home = tempfile::tempdir().unwrap();
    let mosaico = home.path().join("mosaico");
    let mut env = crate::test_env::EnvGuard::set("HOME", home.path());
    env.set_var("MOSAICO_HOME", &mosaico);
    env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
    env.set_var("GOOSE_CONTEXT_LIMIT", "8192");
    env.set_var("PATH", install_fake_goose(home.path(), "1.43.0"));
    install_plugin();
    let error = prepare_launch_env(&mut Vec::new(), "small").unwrap_err();
    assert!(error.to_string().contains("at least 32000"));
}

#[test]
fn old_goose_without_top_of_mind_is_rejected() {
    let home = tempfile::tempdir().unwrap();
    let mut env = crate::test_env::EnvGuard::set("HOME", home.path());
    env.set_var("PATH", install_fake_goose(home.path(), "1.42.0"));
    let error = validate_runtime().unwrap_err();
    assert!(error.to_string().contains("1.43.0 or newer"));
}

#[test]
fn parses_current_goose_version_output() {
    assert_eq!(parse_version(" 1.43.0\n"), Some([1, 43, 0]));
    assert_eq!(parse_version("goose v1.44.2\n"), Some([1, 44, 2]));
}
