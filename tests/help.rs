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
    assert!(stdout.contains("mosaico install"));
    assert!(stdout.contains("mosaico install --all"));
    assert!(!stdout.contains("Usage: mosaico"));
    assert!(!home.path().join(".mosaico/daemon.sock").exists());
}

#[test]
fn bare_invocation_with_installation_is_exactly_agents() {
    let home = installed_codex_home();
    let bare = isolated_command(home.path(), &[]);
    let agents = isolated_command(home.path(), &["agents"]);

    assert!(bare.status.success(), "bare mosaico failed: {bare:?}");
    assert!(agents.status.success(), "mosaico agents failed: {agents:?}");
    assert_eq!(bare.stdout, agents.stdout);
    assert_eq!(bare.stderr, agents.stderr);
    assert!(String::from_utf8_lossy(&bare.stdout).contains("codex"));
}

#[test]
fn removed_launch_subcommand_is_rejected() {
    let home = installed_codex_home();
    let output = isolated_command(home.path(), &["launch"]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand 'launch'"));
}

#[test]
fn explicit_top_level_human_help_remains_contextual() {
    let help = contextual_help(&["--help"], false);

    assert!(help.contains("  sessions"));
    assert!(help.contains("  agents"));
    assert!(!help.contains("  mgmt"));
    assert!(!help.contains("  publish"));
}

#[test]
fn agent_help_hides_operator_agent_management() {
    let help = contextual_help(&["--help"], true);

    assert!(help.contains("  my"));
    assert!(!help.contains("  agents"));
    assert!(!help.contains("  mgmt"));
}
