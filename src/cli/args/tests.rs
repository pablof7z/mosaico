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
    let err = parse_err(&["tenex-edge", "tail", "--live"]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn removed_chat_command_stays_unavailable() {
    let err = parse_err(&["tenex-edge", "chat", "read"]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn removed_project_command_stays_unavailable() {
    let err = parse_err(&["tenex-edge", "project", "list"]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn removed_agent_command_stays_unavailable() {
    let err = parse_err(&["tenex-edge", "agent", "list"]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn removed_config_command_stays_unavailable() {
    let err = parse_err(&["tenex-edge", "config", "providers"]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn removed_session_command_stays_unavailable() {
    let err = parse_err(&["tenex-edge", "session", "end", "--self"]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn removed_channels_alias_stays_unavailable() {
    let err = parse_err(&["tenex-edge", "channels", "list"]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn daemon_stop_parses() {
    let cli = Cli::try_parse_from(["tenex-edge", "daemon", "stop"]).unwrap();

    match cli.cmd {
        Cmd::Daemon(args) => assert!(matches!(args.action, Some(DaemonAction::Stop))),
        _ => panic!("expected daemon stop action"),
    }
}

#[test]
fn daemon_restart_parses() {
    let cli = Cli::try_parse_from(["tenex-edge", "daemon", "restart"]).unwrap();

    match cli.cmd {
        Cmd::Daemon(args) => assert!(matches!(args.action, Some(DaemonAction::Restart))),
        _ => panic!("expected daemon restart action"),
    }
}

#[test]
fn session_end_self_parses() {
    let cli = Cli::try_parse_from(["tenex-edge", "my", "session", "end", "--self"]).unwrap();
    match cli.cmd {
        Cmd::My {
            action:
                MyAction::Session {
                    action: SessionAction::End(args),
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
    let cli = Cli::try_parse_from(["tenex-edge", "my", "session", "kill", "--self"]).unwrap();
    match cli.cmd {
        Cmd::My {
            action:
                MyAction::Session {
                    action: SessionAction::Kill(args),
                },
        } => {
            assert!(args.self_session);
        }
        _ => panic!("expected my session kill action"),
    }
}

#[test]
fn session_kill_without_self_is_rejected() {
    let err = parse_err(&["tenex-edge", "my", "session", "kill"]);

    assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
}

#[test]
fn session_kill_rejects_positional_target() {
    let err = parse_err(&["tenex-edge", "my", "session", "kill", "--self", "te-123"]);

    assert_eq!(err.kind(), ErrorKind::UnknownArgument);
}

#[test]
fn session_kill_rejects_positional_target_without_self() {
    let err = parse_err(&["tenex-edge", "my", "session", "kill", "te-123"]);

    assert!(matches!(
        err.kind(),
        ErrorKind::UnknownArgument | ErrorKind::MissingRequiredArgument
    ));
}

#[test]
fn session_pty_wrap_me_self_parses() {
    let cli =
        Cli::try_parse_from(["tenex-edge", "my", "session", "pty-wrap-me", "--self"]).unwrap();
    match cli.cmd {
        Cmd::My {
            action:
                MyAction::Session {
                    action: SessionAction::PtyWrapMe(args),
                },
        } => {
            assert!(args.self_session);
        }
        _ => panic!("expected my session pty-wrap-me action"),
    }
}

#[test]
fn session_pty_wrap_me_requires_self() {
    let err = parse_err(&["tenex-edge", "my", "session", "pty-wrap-me"]);

    assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
}

#[test]
fn session_pty_wrap_me_rejects_positional_target() {
    let err = parse_err(&[
        "tenex-edge",
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
    let err = parse_err(&["tenex-edge", "my", "session", "pty-wrap-me", "te-123"]);

    assert!(matches!(
        err.kind(),
        ErrorKind::UnknownArgument | ErrorKind::MissingRequiredArgument
    ));
}

#[test]
fn mgmt_config_parses() {
    let cli = Cli::try_parse_from(["tenex-edge", "mgmt", "config", "providers"]).unwrap();
    assert!(matches!(cli.cmd, Cmd::Mgmt { .. }));
}

#[test]
fn my_status_parses_with_topic() {
    let cli = Cli::try_parse_from([
        "tenex-edge",
        "my",
        "status",
        "--topic",
        "Researching MCP improvements around resource allocation",
    ])
    .unwrap();
    assert!(matches!(cli.cmd, Cmd::My { .. }));
}

#[test]
fn mcp_command_parses() {
    let cli = Cli::try_parse_from(["tenex-edge", "mcp"]).unwrap();
    assert!(matches!(cli.cmd, Cmd::Mcp(_)));
}

#[test]
fn mcp_http_command_parses() {
    let cli = Cli::try_parse_from(["tenex-edge", "mcp", "--http", "--port", "9000"]).unwrap();
    assert!(matches!(cli.cmd, Cmd::Mcp(_)));
}

#[test]
fn mgmt_session_list_parses() {
    let cli = Cli::try_parse_from(["tenex-edge", "mgmt", "session", "list"]).unwrap();
    match cli.cmd {
        Cmd::Mgmt {
            action:
                MgmtAction::Session {
                    action: MgmtSessionAction::List,
                },
        } => {}
        _ => panic!("expected mgmt session list command"),
    }
}

#[test]
fn mgmt_session_list_rejects_removed_refresh_flag() {
    let err = parse_err(&[
        "tenex-edge",
        "mgmt",
        "session",
        "list",
        "--refresh-secs",
        "3",
    ]);

    assert_eq!(err.kind(), ErrorKind::UnknownArgument);
}

#[test]
fn removed_top_level_tui_stays_unavailable() {
    let err = parse_err(&["tenex-edge", "tui"]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}
