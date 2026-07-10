use super::*;
use clap::{error::ErrorKind, Parser};

fn parse_err(args: &[&str]) -> clap::Error {
    match crate::cli::args::Cli::try_parse_from(args) {
        Ok(_) => panic!("expected parse failure for {args:?}"),
        Err(err) => err,
    }
}

#[test]
fn agents_list_sessions_filter_still_parses() {
    let cli = crate::cli::args::Cli::try_parse_from([
        "tenex-edge",
        "agents",
        "list-sessions",
        "--agent",
        "claude@laptop",
    ])
    .expect("agents list-sessions parses");

    match cli.cmd {
        crate::cli::args::Cmd::Agents {
            action: Some(AgentsAction::ListSessions { agent, since: None }),
        } => assert_eq!(agent.as_deref(), Some("claude@laptop")),
        _ => panic!("expected agents list-sessions command"),
    }
}

#[test]
fn agent_add_workspace_flag_parses() {
    let cli = crate::cli::args::Cli::try_parse_from([
        "tenex-edge",
        "agent",
        "add",
        "reviewer",
        "--workspace",
        "tenex-edge",
    ])
    .expect("agent add --workspace parses");

    match cli.cmd {
        crate::cli::args::Cmd::Agent {
            action: AgentAction::Add {
                slug, workspaces, ..
            },
        } => {
            assert_eq!(slug, "reviewer");
            assert_eq!(workspaces, vec!["tenex-edge".to_string()]);
        }
        _ => panic!("expected agent add command"),
    }
}

#[test]
fn channel_list_workspace_flags_parse() {
    let one = crate::cli::args::Cli::try_parse_from([
        "tenex-edge",
        "channel",
        "list",
        "--workspace",
        "tenex-edge",
    ])
    .expect("channel list --workspace parses");
    match one.cmd {
        crate::cli::args::Cmd::Channel {
            action:
                ChannelAction::List {
                    workspace,
                    workspaces,
                },
        } => {
            assert_eq!(workspace.as_deref(), Some("tenex-edge"));
            assert!(!workspaces);
        }
        _ => panic!("expected channel list command"),
    }

    let all = crate::cli::args::Cli::try_parse_from([
        "tenex-edge",
        "channel",
        "list",
        "--all-workspaces",
    ])
    .expect("channel list --all-workspaces parses");
    match all.cmd {
        crate::cli::args::Cmd::Channel {
            action:
                ChannelAction::List {
                    workspace,
                    workspaces,
                },
        } => {
            assert_eq!(workspace, None);
            assert!(workspaces);
        }
        _ => panic!("expected channel list command"),
    }
}

#[test]
fn channels_create_help_shows_about_limit() {
    let err = parse_err(&["tenex-edge", "channels", "create", "--help"]);
    let help = err.to_string();

    assert!(help.contains("<PATH>"));
    assert!(help.contains("Short, stable channel description (max 80 chars)"));
}

#[test]
fn channels_create_about_rejects_more_than_80_chars() {
    let too_long = "a".repeat(crate::channel_about::CHANNEL_ABOUT_MAX_CHARS + 1);
    let err = parse_err(&[
        "tenex-edge",
        "channels",
        "create",
        "ops",
        "--about",
        &too_long,
    ]);

    assert_eq!(err.kind(), ErrorKind::ValueValidation);
    assert!(
        err.to_string()
            .contains("--about must be 80 characters or fewer (got 81)"),
        "{err}"
    );
}

#[test]
fn channels_create_parses_hierarchical_path() {
    let cli = crate::cli::args::Cli::try_parse_from([
        "tenex-edge",
        "channels",
        "create",
        "epic/planning",
        "--about",
        "planning room",
        "--agent",
        "codex@laptop",
        "--session",
        "session-1",
    ])
    .expect("channels create parses with positional path");

    match cli.cmd {
        crate::cli::args::Cmd::Channel {
            action:
                ChannelAction::Create {
                    path,
                    about,
                    agents,
                    session,
                },
        } => {
            assert_eq!(path, "epic/planning");
            assert_eq!(about, "planning room");
            assert_eq!(agents, vec!["codex@laptop".to_string()]);
            assert_eq!(session.as_deref(), Some("session-1"));
        }
        _ => panic!("expected channels create command"),
    }
}

#[test]
fn channels_archive_parses_channel_reference() {
    let cli = crate::cli::args::Cli::try_parse_from(["tenex-edge", "channels", "archive", "ops"])
        .expect("channels archive parses");

    match cli.cmd {
        crate::cli::args::Cmd::Channel {
            action:
                ChannelAction::Archive {
                    channel,
                    session: None,
                },
        } => assert_eq!(channel, "ops"),
        _ => panic!("expected channels archive command"),
    }
}

#[test]
fn channels_switch_accepts_explicit_session_anchor() {
    let cli = crate::cli::args::Cli::try_parse_from([
        "tenex-edge",
        "channels",
        "switch",
        "ops",
        "--session",
        "session-1",
    ])
    .expect("channels switch parses with explicit session");

    match cli.cmd {
        crate::cli::args::Cmd::Channel {
            action:
                ChannelAction::Switch {
                    channel,
                    session: Some(session),
                },
        } => {
            assert_eq!(channel, "ops");
            assert_eq!(session, "session-1");
        }
        _ => panic!("expected channels switch command"),
    }
}

#[test]
fn removed_invite_command_stays_unavailable() {
    let err = parse_err(&["tenex-edge", "invite", "--channel", "ops", "--agent", "x"]);
    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn channels_edit_about_parses() {
    let cli = crate::cli::args::Cli::try_parse_from([
        "tenex-edge",
        "channels",
        "edit",
        "epic/planning",
        "--about",
        "new description",
    ])
    .expect("channels edit parses");

    match cli.cmd {
        crate::cli::args::Cmd::Channel {
            action:
                ChannelAction::Edit {
                    channel,
                    about,
                    session: None,
                },
        } => {
            assert_eq!(channel, "epic/planning");
            assert_eq!(about, "new description");
        }
        _ => panic!("expected channels edit command"),
    }
}

#[test]
fn channels_edit_about_rejects_more_than_80_chars() {
    let too_long = "a".repeat(crate::channel_about::CHANNEL_ABOUT_MAX_CHARS + 1);
    let err = parse_err(&[
        "tenex-edge",
        "channels",
        "edit",
        "ops",
        "--about",
        &too_long,
    ]);

    assert_eq!(err.kind(), ErrorKind::ValueValidation);
    assert!(
        err.to_string()
            .contains("--about must be 80 characters or fewer (got 81)"),
        "{err}"
    );
}

#[test]
fn channel_read_help_uses_channel_flag() {
    let err = parse_err(&["tenex-edge", "channel", "read", "--help"]);
    let help = err.to_string();

    assert!(help.contains("--channel <CHANNEL>"));
}

#[test]
fn channel_read_rejects_removed_alias() {
    let err = parse_err(&["tenex-edge", "channel", "read", "--project", "tmp"]);

    assert_eq!(err.kind(), ErrorKind::UnknownArgument);
}

#[test]
fn channel_read_channel_still_parses() {
    let cli = crate::cli::args::Cli::try_parse_from([
        "tenex-edge",
        "channel",
        "read",
        "--channel",
        "ops",
    ])
    .unwrap();

    match cli.cmd {
        crate::cli::args::Cmd::Channel {
            action: ChannelAction::Read { channel, .. },
        } => assert_eq!(channel.as_deref(), Some("ops")),
        _ => panic!("expected channel read command"),
    }
}

#[test]
fn channel_read_alias_via_channels_still_parses() {
    let cli = crate::cli::args::Cli::try_parse_from([
        "tenex-edge",
        "channels",
        "read",
        "--channel",
        "ops",
    ])
    .unwrap();

    assert!(matches!(
        cli.cmd,
        crate::cli::args::Cmd::Channel {
            action: ChannelAction::Read { .. }
        }
    ));
}

#[test]
fn channel_send_accepts_explicit_session_anchor() {
    let cli = crate::cli::args::Cli::try_parse_from([
        "tenex-edge",
        "channel",
        "send",
        "hello",
        "--channel",
        "ops",
        "--session",
        "session-1",
    ])
    .unwrap();

    match cli.cmd {
        crate::cli::args::Cmd::Channel {
            action:
                ChannelAction::Send {
                    message,
                    channel,
                    session,
                    ..
                },
        } => {
            assert_eq!(message.as_deref(), Some("hello"));
            assert_eq!(channel.as_deref(), Some("ops"));
            assert_eq!(session.as_deref(), Some("session-1"));
        }
        _ => panic!("expected channel send command"),
    }
}

#[test]
fn channel_reply_parses_short_id_and_message_flag() {
    let cli = crate::cli::args::Cli::try_parse_from([
        "tenex-edge",
        "channel",
        "reply",
        "abc123",
        "--message",
        "hello",
        "--session",
        "session-1",
    ])
    .unwrap();

    match cli.cmd {
        crate::cli::args::Cmd::Channel {
            action:
                ChannelAction::Reply {
                    id,
                    message_flag,
                    session,
                    ..
                },
        } => {
            assert_eq!(id, "abc123");
            assert_eq!(message_flag.as_deref(), Some("hello"));
            assert_eq!(session.as_deref(), Some("session-1"));
        }
        _ => panic!("expected channel reply command"),
    }
}
