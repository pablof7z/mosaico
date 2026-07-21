use std::process::Command;

fn installed_codex_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let mosaico_home = home.path().join(".mosaico");
    std::fs::create_dir_all(&mosaico_home).unwrap();
    std::fs::write(
        mosaico_home.join("config.json"),
        r#"{"availableHarnesses":[]}"#,
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
        .env("MOSAICO_ISOLATED_HOME_OK", "1")
        .env_remove("MOSAICO_AGENT")
        .output()
        .expect("run isolated mosaico")
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
fn removed_agents_target_is_rejected() {
    let home = installed_codex_home();
    let output = isolated_command(home.path(), &["agents", "codex"]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand 'codex'"));
}

#[test]
fn explicit_top_level_human_help_remains_contextual() {
    let help = contextual_help(&["--help"], false);

    assert!(!help.contains("  sessions"));
    assert!(help.contains("  agents"));
    assert!(help.contains("  setup"));
    assert!(help.contains("  uninstall"));
    assert!(help.contains("  doctor"));
    assert!(!help.contains("  install"));
    assert!(help.contains("without a command"));
    assert!(!help.contains("  mgmt"));
    assert!(!help.contains("  publish"));
}

#[test]
fn agent_help_hides_operator_agent_management() {
    let help = contextual_help(&["--help"], true);

    assert!(help.contains("  my"));
    assert!(help.contains("  doctor"));
    assert!(!help.contains("  agents"));
    assert!(!help.contains("  setup"));
    assert!(!help.contains("  uninstall"));
    assert!(!help.contains("  mgmt"));
}

#[test]
fn doctor_json_reports_unconfigured_home_and_exits_unhealthy() {
    let home = tempfile::tempdir().unwrap();
    let output = isolated_command(home.path(), &["doctor", "--json"]);
    assert!(
        !output.status.success(),
        "unconfigured home must be unhealthy"
    );

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor stdout is one JSON document");
    assert_eq!(report["healthy"], false);
    assert_eq!(report["fix_attempted"], false);
    assert!(report["repairs"].is_array());
    assert!(report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|check| check["name"] == "config.document" && check["status"] == "error"));
    let skill = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "skill.agents")
        .expect("canonical agent skill target is reported");
    assert_eq!(skill["state"], "missing");
    assert_eq!(
        skill["path"],
        home.path()
            .join(".agents/skills/mosaico")
            .display()
            .to_string()
    );
}

#[test]
fn doctor_fix_json_preserves_invalid_identity_and_emits_only_json() {
    let home = tempfile::tempdir().unwrap();
    let mosaico_home = home.path().join(".mosaico");
    std::fs::create_dir_all(&mosaico_home).unwrap();
    let config_path = mosaico_home.join("config.json");
    let original = r#"{"mosaicoPrivateKey":"invalid","unknown":"preserved"}"#;
    std::fs::write(&config_path, original).unwrap();

    let output = isolated_command(home.path(), &["doctor", "--fix", "--json"]);

    assert!(!output.status.success());
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout is one JSON document");
    assert_eq!(report["healthy"], false);
    assert_eq!(report["fix_attempted"], true);
    assert!(report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|check| check["name"] == "repair" && check["status"] == "error"));
    assert_eq!(std::fs::read_to_string(config_path).unwrap(), original);
}
