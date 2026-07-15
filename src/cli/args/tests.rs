use super::*;
use crate::cli::session::SessionAction;
use clap::{error::ErrorKind, Parser};

fn parse_err(args: &[&str]) -> clap::Error {
    match Cli::try_parse_from(args) {
        Ok(_) => panic!("expected parse failure for {args:?}"),
        Err(err) => err,
    }
}

#[test]
fn removed_tail_command_stays_unavailable() {
    let err = parse_err(&["mosaico", "tail", "--live"]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn removed_chat_command_stays_unavailable() {
    let err = parse_err(&["mosaico", "chat", "read"]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn removed_publish_command_stays_unavailable() {
    let err = parse_err(&["mosaico", "publish", "--title", "T"]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn duplicate_top_level_send_surface_stays_unavailable() {
    let err = parse_err(&["mosaico", "send", "hello"]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn removed_project_command_stays_unavailable() {
    let err = parse_err(&["mosaico", "project", "list"]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn removed_agent_command_stays_unavailable() {
    let err = parse_err(&["mosaico", "agent", "list"]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn removed_config_command_stays_unavailable() {
    let err = parse_err(&["mosaico", "config", "providers"]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn removed_session_command_stays_unavailable() {
    let err = parse_err(&["mosaico", "session", "end", "--self"]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn removed_channels_alias_stays_unavailable() {
    let err = parse_err(&["mosaico", "channels", "list"]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn daemon_stop_parses() {
    let cli = Cli::try_parse_from(["mosaico", "daemon", "stop"]).unwrap();

    match cli.cmd {
        Cmd::Daemon(args) => assert!(matches!(args.action, Some(DaemonAction::Stop))),
        _ => panic!("expected daemon stop action"),
    }
}

#[test]
fn daemon_restart_parses() {
    let cli = Cli::try_parse_from(["mosaico", "daemon", "restart"]).unwrap();

    match cli.cmd {
        Cmd::Daemon(args) => assert!(matches!(args.action, Some(DaemonAction::Restart))),
        _ => panic!("expected daemon restart action"),
    }
}

#[test]
fn session_end_self_parses() {
    let cli = Cli::try_parse_from(["mosaico", "my", "session", "end", "--self"]).unwrap();
    match cli.cmd {
        Cmd::My {
            action:
                MyAction::Session {
                    action: Some(SessionAction::End(args)),
                },
        } => {
            assert!(args.self_session);
            assert!(args.session.is_none());
        }
        _ => panic!("expected my session end action"),
    }
}

#[test]
fn session_kill_self_parses() {
    let cli = Cli::try_parse_from(["mosaico", "my", "session", "kill", "--self"]).unwrap();
    match cli.cmd {
        Cmd::My {
            action:
                MyAction::Session {
                    action: Some(SessionAction::Kill(args)),
                },
        } => {
            assert!(args.self_session);
        }
        _ => panic!("expected my session kill action"),
    }
}

#[test]
fn session_kill_without_self_is_rejected() {
    let err = parse_err(&["mosaico", "my", "session", "kill"]);

    assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
}

#[test]
fn session_kill_rejects_positional_target() {
    let err = parse_err(&["mosaico", "my", "session", "kill", "--self", "mosaico-123"]);

    assert_eq!(err.kind(), ErrorKind::UnknownArgument);
}

#[test]
fn session_kill_rejects_positional_target_without_self() {
    let err = parse_err(&["mosaico", "my", "session", "kill", "mosaico-123"]);

    assert!(matches!(
        err.kind(),
        ErrorKind::UnknownArgument | ErrorKind::MissingRequiredArgument
    ));
}

#[test]
fn session_pty_wrap_me_self_parses() {
    let cli = Cli::try_parse_from(["mosaico", "my", "session", "pty-wrap-me", "--self"]).unwrap();
    match cli.cmd {
        Cmd::My {
            action:
                MyAction::Session {
                    action: Some(SessionAction::PtyWrapMe(args)),
                },
        } => {
            assert!(args.self_session);
        }
        _ => panic!("expected my session pty-wrap-me action"),
    }
}

#[test]
fn session_pty_wrap_me_requires_self() {
    let err = parse_err(&["mosaico", "my", "session", "pty-wrap-me"]);

    assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
}

#[test]
fn session_pty_wrap_me_rejects_positional_target() {
    let err = parse_err(&[
        "mosaico",
        "my",
        "session",
        "pty-wrap-me",
        "--self",
        "some-other-session",
    ]);

    assert_eq!(err.kind(), ErrorKind::UnknownArgument);
}

#[test]
fn session_pty_wrap_me_rejects_positional_target_without_self() {
    let err = parse_err(&["mosaico", "my", "session", "pty-wrap-me", "mosaico-123"]);

    assert!(matches!(
        err.kind(),
        ErrorKind::UnknownArgument | ErrorKind::MissingRequiredArgument
    ));
}

#[test]
fn mgmt_config_parses() {
    let cli = Cli::try_parse_from(["mosaico", "mgmt", "config", "providers"]).unwrap();
    assert!(matches!(cli.cmd, Cmd::Mgmt { .. }));
}

#[test]
fn my_session_status_parses_positional_title() {
    let cli = Cli::try_parse_from([
        "mosaico",
        "my",
        "session",
        "status",
        "Researching MCP improvements around resource allocation",
    ])
    .unwrap();
    match cli.cmd {
        Cmd::My {
            action:
                MyAction::Session {
                    action: Some(SessionAction::Status(args)),
                },
        } => assert_eq!(
            args.title,
            "Researching MCP improvements around resource allocation"
        ),
        _ => panic!("expected my session status action"),
    }
}

#[test]
fn my_session_without_action_parses_as_briefing() {
    let cli = Cli::try_parse_from(["mosaico", "my", "session"]).unwrap();
    assert!(matches!(
        cli.cmd,
        Cmd::My {
            action: MyAction::Session { action: None }
        }
    ));
}

#[test]
fn removed_my_status_stays_unavailable() {
    let err = parse_err(&[
        "mosaico",
        "my",
        "status",
        "--topic",
        "Researching MCP improvements",
    ]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn removed_agents_command_stays_unavailable() {
    let err = parse_err(&["mosaico", "agents", "list-sessions"]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn mcp_command_parses() {
    let cli = Cli::try_parse_from(["mosaico", "mcp"]).unwrap();
    assert!(matches!(cli.cmd, Cmd::Mcp(_)));
}

#[test]
fn mcp_http_command_parses() {
    let cli = Cli::try_parse_from(["mosaico", "mcp", "--http", "--port", "9000"]).unwrap();
    assert!(matches!(cli.cmd, Cmd::Mcp(_)));
}

#[test]
fn sessions_parses() {
    let cli = Cli::try_parse_from(["mosaico", "sessions"]).unwrap();
    assert!(matches!(cli.cmd, Cmd::Sessions));
}

#[test]
fn removed_session_and_pty_command_trees_stay_unavailable() {
    assert_eq!(
        parse_err(&["mosaico", "mgmt", "session", "list"]).kind(),
        ErrorKind::InvalidSubcommand
    );
    assert_eq!(
        parse_err(&["mosaico", "pty", "list"]).kind(),
        ErrorKind::InvalidSubcommand
    );
}

#[test]
fn removed_top_level_tui_stays_unavailable() {
    let err = parse_err(&["mosaico", "tui"]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn contextual_help_separates_agent_and_operator_commands() {
    let help = super::command_for_context(true)
        .render_long_help()
        .to_string();

    assert!(!help.contains("  who"), "agent help exposed who:\n{help}");
    assert!(
        !help.contains("  sessions"),
        "agent help exposed sessions:\n{help}"
    );
    for command in ["wait", "dispatch", "my"] {
        assert!(
            help.contains(&format!("  {command}")),
            "agent help omitted {command}:\n{help}"
        );
    }
}

#[test]
fn contextual_help_shows_who_to_humans() {
    let help = super::command_for_context(false)
        .render_long_help()
        .to_string();

    assert!(help.contains("  who"), "human help omitted who:\n{help}");
    assert!(
        help.contains("  sessions"),
        "human help omitted sessions:\n{help}"
    );
    for command in ["wait", "dispatch", "my"] {
        assert!(
            !help.contains(&format!("  {command}")),
            "human help exposed agent-only {command}:\n{help}"
        );
    }
}
