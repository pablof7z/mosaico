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
        &["install", "--harness", "codex"],
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

    let status = run(&binary, &home, &mosaico_home, &["install", "--status"]);
    assert!(status.status.success(), "{}", output_text(&status));
    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(stdout.contains("mosaico skill status"));
    assert!(stdout.contains("installed"));

    let uninstall = run(
        &binary,
        &home,
        &mosaico_home,
        &["install", "--harness", "codex", "--uninstall"],
    );
    assert!(uninstall.status.success(), "{}", output_text(&uninstall));
    assert!(!skill.exists());

    let hooks: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".codex/hooks.json")).unwrap())
            .unwrap();
    assert!(hooks.pointer("/hooks/SessionStart").is_none());
}
