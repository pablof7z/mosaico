use super::*;

#[test]
fn channel_read_help_uses_channel_flag() {
    let err = parse_err(&["mosaico", "channel", "read", "--help"]);
    assert!(err.to_string().contains("--channel <CHANNEL>"));
}

#[test]
fn channel_read_channel_still_parses() {
    let cli =
        crate::cli::args::Cli::try_parse_from(["mosaico", "channel", "read", "--channel", "#ops"])
            .unwrap();
    match cli.cmd.expect("expected channel command") {
        crate::cli::args::Cmd::Channel {
            action: ChannelAction::Read { channel, .. },
        } => assert_eq!(channel.as_deref(), Some("#ops")),
        _ => panic!("expected channel read command"),
    }
}

#[test]
fn channel_search_parses_repeatable_filters() {
    let cli = crate::cli::args::Cli::try_parse_from([
        "mosaico",
        "channel",
        "search",
        "--from",
        "@Pablo",
        "--from",
        "@reviewer",
        "--to",
        "@chief-of-staff",
        "--contains",
        "commit",
        "--channel",
        "#fava/research",
        "--channel",
        "#",
        "--since",
        "1785348600",
        "--until",
        "1785404520",
        "--limit",
        "40",
    ])
    .expect("channel search parses");
    match cli.cmd.expect("expected channel command") {
        crate::cli::args::Cmd::Channel {
            action: ChannelAction::Search(args),
        } => {
            assert_eq!(args.from, ["@Pablo", "@reviewer"]);
            assert_eq!(args.to, ["@chief-of-staff"]);
            assert_eq!(args.contains, ["commit"]);
            assert_eq!(args.channel, ["#fava/research", "#"]);
            assert_eq!(args.since, Some(1_785_348_600));
            assert_eq!(args.until, Some(1_785_404_520));
            assert_eq!(args.limit, Some(40));
            assert_eq!(args.cursor, None);
        }
        _ => panic!("expected channel search command"),
    }

    let cursor = crate::cli::args::Cli::try_parse_from([
        "mosaico", "channel", "search", "--cursor", "opaque",
    ])
    .expect("cursor-only continuation parses");
    match cursor.cmd.expect("expected channel command") {
        crate::cli::args::Cmd::Channel {
            action: ChannelAction::Search(args),
        } => assert_eq!(args.cursor.as_deref(), Some("opaque")),
        _ => panic!("expected channel search command"),
    }
}

#[test]
fn channel_args_reject_slash_prefixed_paths() {
    for args in [
        &["mosaico", "channel", "join", "/fava"][..],
        &["mosaico", "channel", "archive", "/fava/old"][..],
        &[
            "mosaico",
            "channel",
            "create",
            "/fava/child",
            "--about",
            "x",
        ][..],
        &["mosaico", "channel", "search", "--channel", "/fava"][..],
        &[
            "mosaico",
            "dispatch",
            "codex@laptop",
            "--workspace",
            "fava",
            "--channel",
            "/fava",
        ][..],
    ] {
        let err = parse_err(args);
        assert_eq!(err.kind(), ErrorKind::ValueValidation, "{args:?}");
        assert!(
            err.to_string().contains("full path starting with \"#\""),
            "slash form must not be accepted for {args:?}: {err}"
        );
    }
}

#[test]
fn channel_search_rejects_invalid_channel_and_cursor_filters() {
    let channel = parse_err(&["mosaico", "channel", "search", "--channel", "fava"]);
    assert_eq!(channel.kind(), ErrorKind::ValueValidation);
    let cursor_with_filter = parse_err(&[
        "mosaico",
        "channel",
        "search",
        "--cursor",
        "opaque",
        "--contains",
        "commit",
    ]);
    assert_eq!(cursor_with_filter.kind(), ErrorKind::ArgumentConflict);
}
