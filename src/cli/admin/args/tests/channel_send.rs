use super::*;
use clap::Parser;

#[test]
fn accepts_repeated_tags_and_explicit_session_anchor() {
    let cli = crate::cli::args::Cli::try_parse_from([
        "tenex-edge",
        "channel",
        "send",
        "hello",
        "--tag",
        "agent1",
        "--tag",
        "agent2",
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
                    tags,
                    channel,
                    session,
                    ..
                },
        } => {
            assert_eq!(message.as_deref(), Some("hello"));
            assert_eq!(tags, vec!["agent1", "agent2"]);
            assert_eq!(channel.as_deref(), Some("ops"));
            assert_eq!(session.as_deref(), Some("session-1"));
        }
        _ => panic!("expected channel send command"),
    }
}
