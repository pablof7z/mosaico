use std::path::Path;
use std::process::{Command, Output};

/// Setup requires an externally operated relay; the tests never contact it.
const RELAY: &str = "wss://relay.example.invalid";

fn run(binary: &Path, home: &Path, mosaico_home: &Path, args: &[&str]) -> Output {
    Command::new(binary)
        .args(args)
        .current_dir(home)
        .env_clear()
        .env("HOME", home)
        .env("MOSAICO_HOME", mosaico_home)
        .env("PATH", "/usr/bin:/bin")
        .env("SHELL", "/bin/zsh")
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
        &["setup", "--harness", "codex", "--relay", RELAY],
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
    let claude_skill = home.join(".claude/skills/mosaico");
    std::fs::create_dir_all(home.join(".claude/skills")).unwrap();
    #[cfg(unix)]
    if !claude_skill.exists() {
        std::os::unix::fs::symlink(&skill, &claude_skill).unwrap();
    }
    #[cfg(unix)]
    assert!(claude_skill.is_symlink());
    std::fs::write(
        home.join(".claude/settings.json"),
        serde_json::json!({"hooks": {"SessionStart": [hook_group("claude-code")]}}).to_string(),
    )
    .unwrap();
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
        &["setup", "--harness", "codex", "--relay", RELAY],
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

/// Scoped uninstall is the promise that removing one harness costs nothing
/// else: the other harness, its wrapper, the shared skill, and MOSAICO_HOME all
/// survive, and so does foreign content in the shell profile.
#[test]
fn scoped_uninstall_removes_one_harness_and_its_wrapper_only() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("empty-home");
    let mosaico_home = home.join(".mosaico");
    let binary = temp.path().join("mosaico");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_mosaico"), &binary).unwrap();

    let zshrc = home.join(".zshrc");
    std::fs::write(&zshrc, "export EDITOR=vim\n").unwrap();

    let dry = run(
        &binary,
        &home,
        &mosaico_home,
        &[
            "setup",
            "--harness",
            "codex,claude-code",
            "--relay",
            RELAY,
            "--wrap",
            "codex,claude-code",
            "--dry-run",
        ],
    );
    assert!(dry.status.success(), "{}", output_text(&dry));
    assert!(String::from_utf8_lossy(&dry.stdout).contains("would update shell wrappers"));
    assert_eq!(
        std::fs::read_to_string(&zshrc).unwrap(),
        "export EDITOR=vim\n",
        "dry run must not touch the profile"
    );

    let setup = run(
        &binary,
        &home,
        &mosaico_home,
        &[
            "setup",
            "--harness",
            "codex,claude-code",
            "--relay",
            RELAY,
            "--wrap",
            "codex,claude-code",
        ],
    );
    assert!(setup.status.success(), "{}", output_text(&setup));
    let profile = std::fs::read_to_string(&zshrc).unwrap();
    assert!(profile.starts_with("export EDITOR=vim\n"));
    assert!(profile.contains(r#"alias codex="mosaico codex --""#));
    assert!(profile.contains(r#"alias claude="mosaico claude --""#));

    let unknown = run(&binary, &home, &mosaico_home, &["uninstall", "nope"]);
    assert!(!unknown.status.success(), "{}", output_text(&unknown));
    assert_eq!(
        std::fs::read_to_string(&zshrc).unwrap(),
        profile,
        "an unknown harness must fail before any write"
    );

    let scoped = run(&binary, &home, &mosaico_home, &["uninstall", "codex"]);
    assert!(scoped.status.success(), "{}", output_text(&scoped));

    let profile = std::fs::read_to_string(&zshrc).unwrap();
    assert!(profile.starts_with("export EDITOR=vim\n"));
    assert!(!profile.contains("alias codex="));
    assert!(profile.contains(r#"alias claude="mosaico claude --""#));

    let codex: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".codex/hooks.json")).unwrap())
            .unwrap();
    assert!(codex.pointer("/hooks/SessionStart").is_none());
    let claude: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".claude/settings.json")).unwrap())
            .unwrap();
    assert!(claude.pointer("/hooks/SessionStart/0").is_some());

    assert!(home.join(".agents/skills/mosaico/SKILL.md").is_file());
    assert!(mosaico_home.join("config.json").exists());
}
