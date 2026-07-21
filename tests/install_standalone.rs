use std::path::Path;
use std::process::{Command, Output};

fn run(binary: &Path, home: &Path, mosaico_home: &Path, args: &[&str]) -> Output {
    Command::new(binary)
        .args(args)
        .current_dir(home)
        .env_clear()
        .env("HOME", home)
        .env("MOSAICO_HOME", mosaico_home)
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run standalone mosaico binary")
}

fn output_text(output: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn binary_outside_checkout_installs_statuses_and_uninstalls_skill_and_hooks() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("empty-home");
    let mosaico_home = home.join(".mosaico");
    let bin_dir = temp.path().join("release/bin");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&bin_dir).unwrap();

    let binary = bin_dir.join("mosaico");
    std::fs::copy(env!("CARGO_BIN_EXE_mosaico"), &binary).unwrap();

    let install = run(
        &binary,
        &home,
        &mosaico_home,
        &["setup", "--harness", "codex"],
    );
    assert!(install.status.success(), "{}", output_text(&install));

    let skill = home.join(".agents/skills/mosaico");
    assert!(skill.is_dir());
    assert!(!skill.is_symlink());
    assert_eq!(
        std::fs::read_to_string(skill.join("SKILL.md")).unwrap(),
        include_str!("../skills/mosaico/SKILL.md")
    );
    for relative in [
        "agents/openai.yaml",
        "references/channel-creation.md",
        "references/coordination-guide.md",
        "references/cross-workspace.md",
        "references/headless-mode.md",
        "references/identity-and-capabilities.md",
        "references/mcp-chatbot-setup.md",
        "references/public-work-status.md",
    ] {
        assert!(skill.join(relative).is_file(), "missing {relative}");
    }
    let hooks: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".codex/hooks.json")).unwrap())
            .unwrap();
    assert!(hooks.pointer("/hooks/SessionStart/0").is_some());

    let hook_group = |host: &str| {
        serde_json::json!({
            "hooks": [{
                "command": format!("mosaico harness hook {host} --type session-start")
            }]
        })
    };
    std::fs::create_dir_all(home.join(".claude/skills")).unwrap();
    std::fs::write(
        home.join(".claude/settings.json"),
        serde_json::json!({"hooks": {"SessionStart": [hook_group("claude-code")]}}).to_string(),
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&skill, home.join(".claude/skills/mosaico")).unwrap();
    std::fs::create_dir_all(home.join(".grok/hooks")).unwrap();
    std::fs::write(
        home.join(".grok/hooks/mosaico.json"),
        serde_json::json!({"hooks": {"SessionStart": [hook_group("grok")]}}).to_string(),
    )
    .unwrap();
    std::fs::create_dir_all(home.join(".config/opencode/plugin")).unwrap();
    std::fs::write(
        home.join(".config/opencode/plugin/mosaico.ts"),
        "mosaico plugin",
    )
    .unwrap();
    std::fs::create_dir_all(home.join(".hermes/plugins/mosaico")).unwrap();
    std::fs::write(home.join(".hermes/plugins/mosaico/plugin.yaml"), "plugin").unwrap();
    std::fs::write(home.join(".hermes/plugins/mosaico/__init__.py"), "plugin").unwrap();

    let status = run(&binary, &home, &mosaico_home, &["setup", "--status"]);
    assert!(status.status.success(), "{}", output_text(&status));
    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(stdout.contains("mosaico skill status"));
    assert!(stdout.contains("installed"));

    let uninstall = run(&binary, &home, &mosaico_home, &["uninstall"]);
    assert!(uninstall.status.success(), "{}", output_text(&uninstall));
    assert!(!skill.exists());

    let hooks: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".codex/hooks.json")).unwrap())
            .unwrap();
    assert!(hooks.pointer("/hooks/SessionStart").is_none());
    let claude: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".claude/settings.json")).unwrap())
            .unwrap();
    assert!(claude.pointer("/hooks/SessionStart").is_none());
    assert!(!home.join(".claude/skills/mosaico").exists());
    let grok: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.join(".grok/hooks/mosaico.json")).unwrap(),
    )
    .unwrap();
    assert!(grok.pointer("/hooks/SessionStart").is_none());
    assert!(!home.join(".config/opencode/plugin/mosaico.ts").exists());
    assert!(!home.join(".hermes/plugins/mosaico/plugin.yaml").exists());
    assert!(!home.join(".hermes/plugins/mosaico/__init__.py").exists());
    assert!(
        mosaico_home.join("config.json").exists(),
        "state is preserved by default"
    );
}

#[test]
fn explicit_confirmed_purge_removes_only_mosaico_state() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("empty-home");
    let mosaico_home = home.join(".mosaico");
    let binary = temp.path().join("mosaico");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_mosaico"), &binary).unwrap();
    std::fs::write(home.join("keep.txt"), "keep").unwrap();

    let setup = run(
        &binary,
        &home,
        &mosaico_home,
        &["setup", "--harness", "codex"],
    );
    assert!(setup.status.success(), "{}", output_text(&setup));
    let uninstall = run(
        &binary,
        &home,
        &mosaico_home,
        &["uninstall", "--purge-state", "--yes"],
    );

    assert!(uninstall.status.success(), "{}", output_text(&uninstall));
    assert!(!mosaico_home.exists());
    assert_eq!(
        std::fs::read_to_string(home.join("keep.txt")).unwrap(),
        "keep"
    );
}
