use super::*;

#[test]
fn native_resume_parses_with_optional_workspace() {
    let cli = Cli::try_parse_from([
        "mosaico",
        "resume",
        "019f7f5c-575d-7640-958d-e7428d4d77b0",
        "--workspace",
        "/work/mosaico",
    ])
    .unwrap();

    match cli.cmd.expect("expected resume command") {
        Cmd::Resume(args) => {
            assert_eq!(args.harness_id, "019f7f5c-575d-7640-958d-e7428d4d77b0");
            assert_eq!(
                args.workspace.as_deref(),
                Some(std::path::Path::new("/work/mosaico"))
            );
        }
        _ => panic!("expected native resume command"),
    }
}

#[test]
fn contextual_help_separates_agent_and_operator_commands() {
    let help = super::super::command_for_context(true)
        .render_long_help()
        .to_string();

    for hidden in ["who", "sessions", "resume"] {
        assert!(
            !help.contains(&format!("  {hidden}")),
            "agent help exposed {hidden}:\n{help}"
        );
    }
    for command in ["wait", "dispatch", "my"] {
        assert!(
            help.contains(&format!("  {command}")),
            "agent help omitted {command}:\n{help}"
        );
    }
}

#[test]
fn contextual_help_shows_current_operator_commands_to_humans() {
    let help = super::super::command_for_context(false)
        .render_long_help()
        .to_string();

    for visible in ["who", "resume", "agents"] {
        assert!(
            help.contains(&format!("  {visible}")),
            "human help omitted {visible}:\n{help}"
        );
    }
    assert!(
        !help.contains("  sessions"),
        "removed command leaked into help:\n{help}"
    );
    for hidden in ["wait", "dispatch", "my"] {
        assert!(
            !help.contains(&format!("  {hidden}")),
            "human help exposed agent-only {hidden}:\n{help}"
        );
    }
}
