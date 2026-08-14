use std::process::Command;

fn installed_codex_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let bin_dir = home.path().join("bin");
    let mosaico_home = home.path().join(".mosaico");
    std::fs::create_dir_all(&bin_dir).unwrap();
    std::fs::create_dir_all(&mosaico_home).unwrap();
    let codex = bin_dir.join("codex");
    std::fs::write(&codex, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::write(
        mosaico_home.join("config.json"),
        r#"{"availableHarnesses":[],"relays":["ws://127.0.0.1:1"]}"#,
    )
    .unwrap();
    let codex_home = home.path().join(".codex");
    std::fs::create_dir_all(&codex_home).unwrap();
    let group = |hook_type: &str| {
        serde_json::json!([{
            "hooks": [{
                "command": format!("mosaico harness hook codex --type {hook_type}")
            }]
        }])
    };
    std::fs::write(
        codex_home.join("hooks.json"),
        serde_json::json!({
            "hooks": {
                "SessionStart": group("session-start"),
                "UserPromptSubmit": group("user-prompt-submit"),
                "PreToolUse": group("pre-tool-use"),
                "PostToolUse": group("post-tool-use"),
                "Stop": group("stop")
            }
        })
        .to_string(),
    )
    .unwrap();
    home
}

fn isolated_command(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mosaico"))
        .args(args)
        .env("HOME", home)
        .env("MOSAICO_HOME", home.join(".mosaico"))
        .env_remove("MOSAICO")
        .env("MOSAICO_ISOLATED_HOME_OK", "1")
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", home.join("bin").display()),
        )
        .env_remove("MOSAICO_AGENT")
        .output()
        .expect("run isolated mosaico")
}

fn named_command(home: &std::path::Path, instance: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mosaico"))
        .args(args)
        .env("HOME", home)
        .env("MOSAICO", instance)
        .env_remove("MOSAICO_HOME")
        .env_remove("MOSAICO_CONFIG")
        .env("MOSAICO_ISOLATED_HOME_OK", "1")
        .env_remove("MOSAICO_AGENT")
        .output()
        .expect("run named Mosaico instance")
}

fn contextual_help(args: &[&str], agent: bool) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mosaico"));
    command.args(args);
    if agent {
        command.env("MOSAICO_AGENT", "test-agent");
    } else {
        command.env_remove("MOSAICO_AGENT");
    }
    let output = command.output().expect("run mosaico help");

    assert!(output.status.success(), "help failed: {output:?}");
    String::from_utf8(output.stdout).expect("help is UTF-8")
}

#[test]
fn bare_invocation_without_installation_shows_install_guide() {
    let home = tempfile::tempdir().unwrap();
    let output = isolated_command(home.path(), &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "bare mosaico failed: {output:?}");
    assert!(stdout.contains("mosaico setup"));
    assert!(stdout.contains("mosaico setup --all"));
    assert!(!stdout.contains("Usage: mosaico"));
    assert!(!home.path().join(".mosaico/daemon.sock").exists());
}

#[test]
fn bare_invocation_with_installation_shows_sessions_and_agents() {
    let home = installed_codex_home();
    let bare = isolated_command(home.path(), &[]);
    let stopped = isolated_command(home.path(), &["daemon", "stop"]);

    assert!(bare.status.success(), "bare mosaico failed: {bare:?}");
    let stdout = String::from_utf8_lossy(&bare.stdout);
    assert!(stdout.contains("Sessions"), "{stdout}");
    assert!(stdout.contains("Start a session"), "{stdout}");
    assert!(stdout.contains("codex"), "{stdout}");

    assert!(
        stopped.status.success(),
        "daemon teardown failed: {stopped:?}"
    );
}

#[test]
fn explicit_top_level_human_help_remains_contextual() {
    let help = contextual_help(&["--help"], false);

    assert!(!help.contains("  sessions"));
    assert!(help.contains("  agents"));
    assert!(help.contains("  setup"));
    assert!(help.contains("  uninstall"));
    assert!(help.contains("without a command"));
    assert!(!help.contains("  mgmt"));
    assert!(!help.contains("  publish"));
}

#[test]
fn agent_help_hides_operator_agent_management() {
    let help = contextual_help(&["--help"], true);

    assert!(help.contains("  my"));
    // `--yes-lets-move` is handed to an agent by the topology nudge at the
    // moment it applies, never discovered from help — hidden in both contexts.
    assert!(!help.contains("--yes-lets-move"));
    assert!(!help.contains("  agents"));
    assert!(!help.contains("  setup"));
    assert!(!help.contains("  uninstall"));
    assert!(!help.contains("  mgmt"));
}

#[test]
fn invalid_instance_name_fails_without_touching_any_instance_home() {
    let home = tempfile::tempdir().unwrap();
    let output = named_command(home.path(), "../relay1", &["setup", "--status"]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid MOSAICO instance name"));
    assert!(!home.path().join(".mosaico").exists());
    assert!(!home.path().join(".mosaico-instances").exists());
}

#[test]
fn named_selector_rejects_path_override_instead_of_choosing_precedence() {
    let home = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mosaico"))
        .args(["setup", "--status"])
        .env("HOME", home.path())
        .env("MOSAICO", "relay1")
        .env("MOSAICO_HOME", home.path().join("override"))
        .env_remove("MOSAICO_CONFIG")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("MOSAICO cannot be combined with MOSAICO_HOME"));
    assert!(!home.path().join(".mosaico-instances").exists());
}
