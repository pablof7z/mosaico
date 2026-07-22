use super::*;
use crate::cli::session::SessionAction;
use clap::{error::ErrorKind, Parser};

#[path = "tests/agents.rs"]
mod agents;
#[path = "tests/operator_resume.rs"]
mod operator_resume;

fn parse_err(args: &[&str]) -> clap::Error {
    match Cli::try_parse_from(args) {
        Ok(_) => panic!("expected parse failure for {args:?}"),
        Err(err) => err,
    }
}

#[test]
fn yes_lets_move_parses_as_top_level_acceptance() {
    let cli = Cli::try_parse_from([
        "mosaico",
        "--yes-lets-move",
        "harness-support",
        "Coordinate harness integration support",
    ])
    .expect("acceptance flag should parse");

    assert_eq!(
        cli.yes_lets_move.as_deref(),
        Some(
            [
                "harness-support".to_string(),
                "Coordinate harness integration support".to_string(),
            ]
            .as_slice()
        )
    );
    assert!(cli.cmd.is_none());
}

#[test]
fn yes_lets_move_rejects_a_missing_about() {
    let err = parse_err(&["mosaico", "--yes-lets-move", "harness-support"]);

    assert_eq!(err.kind(), ErrorKind::WrongNumberOfValues);
}

#[test]
fn unknown_top_level_command_routes_to_direct_fallback() {
    let cli = Cli::try_parse_from(["mosaico", "help", "--", "--yolo"]).unwrap();

    match cli.cmd.expect("expected fallback") {
        Cmd::Fallback(args) => assert_eq!(args, ["help", "--", "--yolo"]),
        _ => panic!("expected fallback command"),
    }
}

#[test]
fn daemon_stop_parses() {
    let cli = Cli::try_parse_from(["mosaico", "daemon", "stop"]).unwrap();

    match cli.cmd.expect("expected daemon command") {
        Cmd::Daemon(args) => assert!(matches!(args.action, Some(DaemonAction::Stop))),
        _ => panic!("expected daemon stop action"),
    }
}

#[test]
fn daemon_restart_parses() {
    let cli = Cli::try_parse_from(["mosaico", "daemon", "restart"]).unwrap();

    match cli.cmd.expect("expected daemon command") {
        Cmd::Daemon(args) => assert!(matches!(args.action, Some(DaemonAction::Restart))),
        _ => panic!("expected daemon restart action"),
    }
}

#[test]
fn session_end_self_parses() {
    let cli = Cli::try_parse_from(["mosaico", "my", "session", "end", "--self"]).unwrap();
    match cli.cmd.expect("expected my command") {
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
    match cli.cmd.expect("expected my command") {
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
    match cli.cmd.expect("expected my command") {
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
fn my_session_status_parses_positional_title() {
    let cli = Cli::try_parse_from([
        "mosaico",
        "my",
        "session",
        "status",
        "Researching MCP improvements around resource allocation",
    ])
    .unwrap();
    match cli.cmd.expect("expected my command") {
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
        Some(Cmd::My {
            action: MyAction::Session { action: None }
        })
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
fn mcp_command_parses() {
    let cli = Cli::try_parse_from(["mosaico", "mcp"]).unwrap();
    assert!(matches!(cli.cmd, Some(Cmd::Mcp(_))));
}

#[test]
fn mcp_http_command_parses() {
    let cli = Cli::try_parse_from(["mosaico", "mcp", "--http", "--port", "9000"]).unwrap();
    assert!(matches!(cli.cmd, Some(Cmd::Mcp(_))));
}

#[test]
fn relay_command_parses_before_external_agent_fallback() {
    let cli = Cli::try_parse_from(["mosaico", "relay", "--port", "19888"]).unwrap();
    assert!(matches!(cli.cmd, Some(Cmd::Relay(_))));
}

#[test]
fn bare_invocation_has_no_subcommand() {
    let cli = Cli::try_parse_from(["mosaico"]).unwrap();
    assert!(cli.cmd.is_none());
}

#[test]
fn removed_install_command_is_not_a_setup_alias() {
    let cli = Cli::try_parse_from(["mosaico", "install", "--status"]).unwrap();
    assert!(matches!(cli.cmd, Some(Cmd::Fallback(_))));
}
