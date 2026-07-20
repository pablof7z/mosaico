use super::*;
use clap::Parser;

#[test]
fn accepts_repeated_tags_and_explicit_session_anchor() {
    let cli = crate::cli::args::Cli::try_parse_from([
        "mosaico",
        "channel",
        "send",
        "hello",
        "--tag",
        "agent1",
        "--tag",
        "agent2",
        "--attach",
        "diagram=out/diagram.png",
        "--attach",
        "logs=out/build.log",
        "--force",
        "--channel",
        "ops",
        "--session",
        "session-1",
        "--wait",
        "600",
    ])
    .unwrap();

    match cli.cmd.expect("expected channel command") {
        crate::cli::args::Cmd::Channel {
            action:
                ChannelAction::Send {
                    message,
                    attachments,
                    tags,
                    force,
                    channel,
                    session,
                    wait,
                    ..
                },
        } => {
            assert_eq!(message.as_deref(), Some("hello"));
            assert_eq!(attachments.len(), 2);
            assert_eq!(attachments[0].label, "diagram");
            assert_eq!(
                attachments[0].path,
                std::path::PathBuf::from("out/diagram.png")
            );
            assert_eq!(attachments[1].label, "logs");
            assert_eq!(tags, vec!["agent1", "agent2"]);
            assert!(force);
            assert_eq!(channel.as_deref(), Some("ops"));
            assert_eq!(session.as_deref(), Some("session-1"));
            assert_eq!(wait, Some(600));
        }
        _ => panic!("expected channel send command"),
    }
}
