use super::*;
use clap::{error::ErrorKind, Parser};

#[test]
fn wait_seconds_rejects_zero_and_non_numbers() {
    assert!(parse_wait_seconds("0").is_err());
    assert!(parse_wait_seconds("soon").is_err());
    assert_eq!(parse_wait_seconds("600").unwrap(), 600);
}

#[test]
fn top_level_wait_parses_repeated_channels_and_author() {
    let cli = crate::cli::args::Cli::try_parse_from([
        "tenex-edge",
        "wait",
        "60",
        "--channel",
        "x",
        "--channel",
        "y",
        "--from",
        "agent5",
    ])
    .unwrap();

    match cli.cmd {
        crate::cli::args::Cmd::Wait(args) => {
            assert_eq!(args.timeout_secs, 60);
            assert_eq!(args.channels, ["x", "y"]);
            assert_eq!(args.from.as_deref(), Some("agent5"));
        }
        _ => panic!("expected wait command"),
    }
}

#[test]
fn top_level_wait_without_channels_parses_as_active_channel_union() {
    let cli = crate::cli::args::Cli::try_parse_from(["tenex-edge", "wait", "10"]).unwrap();
    match cli.cmd {
        crate::cli::args::Cmd::Wait(args) => assert!(args.channels.is_empty()),
        _ => panic!("expected wait command"),
    }
}

#[test]
fn wait_has_no_json_mode() {
    let error = match crate::cli::args::Cli::try_parse_from(["tenex-edge", "wait", "10", "--json"])
    {
        Err(error) => error,
        Ok(_) => panic!("wait must keep one agent-native output mode"),
    };
    assert_eq!(error.kind(), ErrorKind::UnknownArgument);
}

#[test]
fn agent_native_wait_renderers_use_one_tenex_edge_envelope() {
    let message = crate::injection::render_agent_message("root.x", "agent5", "abcdef123", "done");
    assert!(message.starts_with("<tenex-edge>"));
    assert!(message.contains("<channel ref=\"root.x\">"));
    assert!(message.contains("<message from=\"@agent5\" id=\"abcdef\">done</message>"));

    let timeout = crate::injection::render_agent_wait_timeout(60, &["root.x", "root.y"]);
    assert!(timeout.starts_with("<tenex-edge>"));
    assert!(timeout.contains("<wait outcome=\"timeout\" after=\"60s\">"));
    assert!(timeout.contains("<channel ref=\"root.y\" />"));
}
