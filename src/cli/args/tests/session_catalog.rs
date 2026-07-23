use super::*;

#[test]
fn list_parses_pagination_and_all_workspaces() {
    let cli = Cli::try_parse_from([
        "mosaico",
        "session",
        "list",
        "--all-workspaces",
        "--limit",
        "50",
        "--offset",
        "100",
        "--json",
    ])
    .unwrap();

    assert!(matches!(
        cli.cmd,
        Some(Cmd::Session {
            action: SessionCatalogAction::List(_)
        })
    ));
}

#[test]
fn find_parses_query_and_filters() {
    let cli = Cli::try_parse_from([
        "mosaico",
        "session",
        "find",
        "buzz mosaico",
        "--workspace",
        "mosaico",
        "--state",
        "offline",
        "--resumable",
        "--since",
        "5d",
    ])
    .unwrap();

    assert!(matches!(
        cli.cmd,
        Some(Cmd::Session {
            action: SessionCatalogAction::Find(_)
        })
    ));
}

#[test]
fn list_rejects_workspace_with_all_workspaces() {
    let err = parse_err(&[
        "mosaico",
        "session",
        "list",
        "--workspace",
        "mosaico",
        "--all-workspaces",
    ]);

    assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
}

#[test]
fn limit_is_bounded() {
    let err = parse_err(&["mosaico", "session", "list", "--limit", "201"]);

    assert_eq!(err.kind(), ErrorKind::ValueValidation);
}
