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
        "mosaico",
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

    match cli.cmd.expect("expected wait command") {
        crate::cli::args::Cmd::Wait(args) => {
            assert_eq!(args.timeout_secs, 60);
            assert_eq!(args.channels, ["x", "y"]);
            assert_eq!(args.from.as_deref(), Some("agent5"));
        }
        _ => panic!("expected wait command"),
    }
}

#[test]
fn top_level_wait_without_channels_parses_as_joined_channel_union() {
    let cli = crate::cli::args::Cli::try_parse_from(["mosaico", "wait", "10"]).unwrap();
    match cli.cmd.expect("expected wait command") {
        crate::cli::args::Cmd::Wait(args) => assert!(args.channels.is_empty()),
        _ => panic!("expected wait command"),
    }
}

#[test]
fn wait_has_no_json_mode() {
    let error = match crate::cli::args::Cli::try_parse_from(["mosaico", "wait", "10", "--json"]) {
        Err(error) => error,
        Ok(_) => panic!("wait must keep one agent-native output mode"),
    };
    assert_eq!(error.kind(), ErrorKind::UnknownArgument);
}

#[test]
fn agent_native_wait_renderers_use_one_mosaico_envelope() {
    let message = render_wait_message(
        &serde_json::json!({
            "channel": "/root/x",
            "from_ref": "agent5",
            "recipient_refs": ["reviewer"],
            "event_id": "abcdef123",
            "body": "done",
            "attachment_dir": "/tmp/mosaico-files/abcdef",
            "created_at": 100,
        }),
        160,
    );
    assert!(message.starts_with("<mosaico>"));
    assert!(message.contains("<channel ref=\"/root/x\">"));
    assert!(message.contains(
        "<message from=\"@agent5\" id=\"abcdef\" for=\"@reviewer\" attachment-dir=\"/tmp/mosaico-files/abcdef\" age=\"1m\">done</message>"
    ));

    let timeout = crate::injection::render_agent_wait_timeout(60, &["/root/x", "/root/y"]);
    assert!(timeout.starts_with("<mosaico>"));
    assert!(timeout.contains("<wait outcome=\"timeout\" after=\"60s\">"));
    assert!(timeout.contains("<channel ref=\"/root/y\" />"));
}

#[test]
fn direct_delivery_and_wait_share_the_exact_message_element() {
    let store = crate::state::Store::open_memory().unwrap();
    store.upsert_channel("x", "x", "", "", 1).unwrap();
    let row = crate::state::InboxRow {
        event_id: "abcdef123".into(),
        target_pubkey: "pk-target".into(),
        state: "pending".into(),
        from_pubkey: "pk-sender".into(),
        channel_h: "x".into(),
        body: "done & checked".into(),
        created_at: 100,
        delivered_at: 0,
        attachment_dir: "/tmp/mosaico-files/abcdef".into(),
    };
    let direct = crate::injection::render_terminal_mention(&store, &[row], &[], 160, true).unwrap();
    let waited = render_wait_message(
        &serde_json::json!({
            "channel": "/x",
            "from_ref": "pk-sende",
            "recipient_refs": ["pk-targe"],
            "event_id": "abcdef123",
            "body": "done & checked",
            "attachment_dir": "/tmp/mosaico-files/abcdef",
            "created_at": 100,
        }),
        160,
    );

    assert_eq!(message_element(&direct), message_element(&waited));
}

fn message_element(document: &str) -> &str {
    let start = document.find("<message").expect("message start");
    let end = document.find("</message>").expect("message end") + "</message>".len();
    &document[start..end]
}
