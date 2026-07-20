use super::*;
use clap::{error::ErrorKind, Parser};

mod channel_send;

fn parse_err(args: &[&str]) -> clap::Error {
    match crate::cli::args::Cli::try_parse_from(args) {
        Ok(_) => panic!("expected parse failure for {args:?}"),
        Err(err) => err,
    }
}

#[test]
fn removed_agent_add_command_stays_unavailable() {
    let err = parse_err(&[
        "mosaico",
        "agent",
        "add",
        "reviewer",
        "--workspace",
        "mosaico",
    ]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn channel_list_workspace_flags_parse() {
    let one = crate::cli::args::Cli::try_parse_from([
        "mosaico",
        "channel",
        "list",
        "--workspace",
        "mosaico",
    ])
    .expect("channel list --workspace parses");
    match one.cmd.expect("expected channel command") {
        crate::cli::args::Cmd::Channel {
            action:
                ChannelAction::List {
                    workspace,
                    workspaces,
                },
        } => {
            assert_eq!(workspace.as_deref(), Some("mosaico"));
            assert!(!workspaces);
        }
        _ => panic!("expected channel list command"),
    }

    let all =
        crate::cli::args::Cli::try_parse_from(["mosaico", "channel", "list", "--all-workspaces"])
            .expect("channel list --all-workspaces parses");
    match all.cmd.expect("expected channel command") {
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
fn channel_create_help_shows_about_limit() {
    let err = parse_err(&["mosaico", "channel", "create", "--help"]);
    let help = err.to_string();

    assert!(help.contains("<PATH>"));
    assert!(help.contains("Short, stable channel description (max 80 chars)"));
}

#[test]
fn channel_create_about_rejects_more_than_80_chars() {
    let too_long = "a".repeat(crate::channel_about::CHANNEL_ABOUT_MAX_CHARS + 1);
    let err = parse_err(&["mosaico", "channel", "create", "ops", "--about", &too_long]);

    assert_eq!(err.kind(), ErrorKind::ValueValidation);
    assert!(
        err.to_string()
            .contains("--about must be 80 characters or fewer (got 81)"),
        "{err}"
    );
}

#[test]
fn channel_create_parses_hierarchical_path() {
    let cli = crate::cli::args::Cli::try_parse_from([
        "mosaico",
        "channel",
        "create",
        "epic.planning",
        "--about",
        "planning room",
        "--agent",
        "codex@laptop",
        "--session",
        "session-1",
    ])
    .expect("channel create parses with positional path");

    match cli.cmd.expect("expected channel command") {
        crate::cli::args::Cmd::Channel {
            action:
                ChannelAction::Create {
                    path,
                    about,
                    agents,
                    session,
                },
        } => {
            assert_eq!(path, "epic.planning");
            assert_eq!(about, "planning room");
            assert_eq!(agents, vec!["codex@laptop".to_string()]);
            assert_eq!(session.as_deref(), Some("session-1"));
        }
        _ => panic!("expected channel create command"),
    }
}

#[test]
fn channel_archive_parses_channel_reference() {
    let cli = crate::cli::args::Cli::try_parse_from(["mosaico", "channel", "archive", "ops"])
        .expect("channel archive parses");

    match cli.cmd.expect("expected channel command") {
        crate::cli::args::Cmd::Channel {
            action:
                ChannelAction::Archive {
                    channel,
                    session: None,
                },
        } => assert_eq!(channel, "ops"),
        _ => panic!("expected channel archive command"),
    }
}

#[test]
fn channel_switch_accepts_explicit_session_anchor() {
    let cli = crate::cli::args::Cli::try_parse_from([
        "mosaico",
        "channel",
        "switch",
        "ops",
        "--session",
        "session-1",
    ])
    .expect("channel switch parses with explicit session");

    match cli.cmd.expect("expected channel command") {
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
        _ => panic!("expected channel switch command"),
    }
}

#[test]
fn removed_invite_command_stays_unavailable() {
    let err = parse_err(&["mosaico", "invite", "--channel", "ops", "--agent", "x"]);
    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn channel_edit_about_parses() {
    let cli = crate::cli::args::Cli::try_parse_from([
        "mosaico",
        "channel",
        "edit",
        "epic.planning",
        "--about",
        "new description",
    ])
    .expect("channel edit parses");

    match cli.cmd.expect("expected channel command") {
        crate::cli::args::Cmd::Channel {
            action:
                ChannelAction::Edit {
                    channel,
                    about,
                    session: None,
                },
        } => {
            assert_eq!(channel, "epic.planning");
            assert_eq!(about, "new description");
        }
        _ => panic!("expected channel edit command"),
    }
}

#[test]
fn channel_edit_about_rejects_more_than_80_chars() {
    let too_long = "a".repeat(crate::channel_about::CHANNEL_ABOUT_MAX_CHARS + 1);
    let err = parse_err(&["mosaico", "channel", "edit", "ops", "--about", &too_long]);

    assert_eq!(err.kind(), ErrorKind::ValueValidation);
    assert!(
        err.to_string()
            .contains("--about must be 80 characters or fewer (got 81)"),
        "{err}"
    );
}

#[test]
fn channel_read_help_uses_channel_flag() {
    let err = parse_err(&["mosaico", "channel", "read", "--help"]);
    let help = err.to_string();

    assert!(help.contains("--channel <CHANNEL>"));
}

#[test]
fn channel_read_rejects_removed_alias() {
    let err = parse_err(&["mosaico", "channel", "read", "--project", "tmp"]);

    assert_eq!(err.kind(), ErrorKind::UnknownArgument);
}

#[test]
fn channel_read_channel_still_parses() {
    let cli =
        crate::cli::args::Cli::try_parse_from(["mosaico", "channel", "read", "--channel", "ops"])
            .unwrap();

    match cli.cmd.expect("expected channel command") {
        crate::cli::args::Cmd::Channel {
            action: ChannelAction::Read { channel, .. },
        } => assert_eq!(channel.as_deref(), Some("ops")),
        _ => panic!("expected channel read command"),
    }
}

#[test]
fn channel_read_alias_via_channels_is_removed() {
    let err = parse_err(&["mosaico", "channels", "read", "--channel", "ops"]);

    assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
}

#[test]
fn channel_reply_parses_short_id_and_message_flag() {
    let cli = crate::cli::args::Cli::try_parse_from([
        "mosaico",
        "channel",
        "reply",
        "abc123",
        "--message",
        "see [trace]",
        "--attach",
        "trace=out/trace.bin",
        "--session",
        "session-1",
    ])
    .unwrap();

    match cli.cmd.expect("expected channel command") {
        crate::cli::args::Cmd::Channel {
            action:
                ChannelAction::Reply {
                    id,
                    message_flag,
                    attachments,
                    session,
                    ..
                },
        } => {
            assert_eq!(id, "abc123");
            assert_eq!(message_flag.as_deref(), Some("see [trace]"));
            assert_eq!(attachments.len(), 1);
            assert_eq!(attachments[0].label, "trace");
            assert_eq!(session.as_deref(), Some("session-1"));
        }
        _ => panic!("expected channel reply command"),
    }
}

#[test]
fn channel_react_parses_id_emoji_and_session() {
    let cli = crate::cli::args::Cli::try_parse_from([
        "mosaico",
        "channel",
        "react",
        "abc123",
        "👍",
        "--session",
        "session-1",
    ])
    .unwrap();

    match cli.cmd.expect("expected channel command") {
        crate::cli::args::Cmd::Channel {
            action: ChannelAction::React { id, emoji, session },
        } => {
            assert_eq!(id, "abc123");
            assert_eq!(emoji, "👍");
            assert_eq!(session.as_deref(), Some("session-1"));
        }
        _ => panic!("expected channel react command"),
    }
}
