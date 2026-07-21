use super::*;

#[test]
fn claude_resume_appends_the_native_id_after_bundle_args() {
    let command = build_driver_resume_command(
        &["claude".into(), "--dangerously-skip-permissions".into()],
        ResumeMechanism::AppendFlag("--resume"),
        "02ff0867-a7bb-4254-a36e-37081ccc3b51",
        "developer",
    )
    .unwrap();

    assert_eq!(
        command,
        [
            "claude",
            "--dangerously-skip-permissions",
            "--resume",
            "02ff0867-a7bb-4254-a36e-37081ccc3b51",
        ]
    );
}

#[test]
fn goose_resume_appends_both_required_flags() {
    let command = build_driver_resume_command(
        &["goose".into(), "session".into()],
        ResumeMechanism::AppendFlags(&["--resume", "--session-id"]),
        "20260721_9",
        "goose",
    )
    .unwrap();

    assert_eq!(
        command,
        ["goose", "session", "--resume", "--session-id", "20260721_9"]
    );
}

#[test]
fn codex_resume_inserts_the_subcommand_before_bundle_args() {
    let command = build_driver_resume_command(
        &["codex".into(), "--yolo".into()],
        ResumeMechanism::Subcommand("resume"),
        "019f7f5c-575d-7640-958d-e7428d4d77b0",
        "codex",
    )
    .unwrap();

    assert_eq!(
        command,
        [
            "codex",
            "resume",
            "019f7f5c-575d-7640-958d-e7428d4d77b0",
            "--yolo",
        ]
    );
}
