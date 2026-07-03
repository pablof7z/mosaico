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
fn channels_create_help_shows_about_limit() {
    let err = parse_err(&["tenex-edge", "channels", "create", "--help"]);
    let help = err.to_string();

    assert!(help.contains("Short, stable channel description (max 80 chars)"));
}

#[test]
fn channels_create_about_rejects_more_than_80_chars() {
    let too_long = "a".repeat(crate::channel_about::CHANNEL_ABOUT_MAX_CHARS + 1);
    let err = parse_err(&[
        "tenex-edge",
        "channels",
        "create",
        "--name",
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
fn invite_requires_agent_or_session_and_preserves_xor() {
    let missing = parse_err(&["tenex-edge", "invite", "--channel", "ops"]);
    assert_eq!(
        missing.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );

    let both = parse_err(&[
        "tenex-edge",
        "invite",
        "--channel",
        "ops",
        "--agent",
        "claude",
        "--session",
        "s1",
    ]);
    assert_eq!(both.kind(), clap::error::ErrorKind::ArgumentConflict);

    let cli = crate::cli::args::Cli::try_parse_from([
        "tenex-edge",
        "invite",
        "--channel",
        "ops",
        "--agent",
        "claude@laptop",
    ])
    .expect("invite with agent parses");

    match cli.cmd {
        crate::cli::args::Cmd::Invite(args) => {
            assert_eq!(args.channel, "ops");
            assert_eq!(args.agent.as_deref(), Some("claude@laptop"));
            assert_eq!(args.session, None);
        }
        _ => panic!("expected invite command"),
    }
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
        crate::cli::args::Cmd::Channels {
            action: ChannelsAction::Edit { channel, about },
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
