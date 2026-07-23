use super::*;
use crate::cli::interactive::session_picker::data::WorkspaceGroup;

fn row(
    handle: &str,
    workspace: &str,
    title: &str,
    state: SessionState,
    last_seen: u64,
) -> SessionRow {
    SessionRow {
        pubkey: format!("pk-{handle}"),
        npub: format!("npub-{handle}"),
        handle: handle.into(),
        agent: "codex".into(),
        workspaces: vec![WorkspaceGroup {
            id: workspace.into(),
            name: workspace.into(),
            path: Some(format!("/work/{workspace}")),
            ..WorkspaceGroup::default()
        }],
        title: title.into(),
        state,
        created_at: last_seen.saturating_sub(10),
        last_seen,
        state_since: last_seen,
        running: state != SessionState::Offline,
        ..SessionRow::default()
    }
}

fn common() -> CommonArgs {
    CommonArgs {
        state: None,
        resumable: false,
        since: None,
        limit: 20,
        offset: 0,
        json: false,
    }
}

#[test]
fn list_is_recent_first_and_scoped_to_workspace() {
    let rows = vec![
        row("old", "mosaico", "Earlier work", SessionState::Offline, 20),
        row("other", "ultra", "OCR", SessionState::Idle, 90),
        row("new", "mosaico", "Current work", SessionState::Idle, 80),
    ];

    let page = query(rows, Mode::List, Some("mosaico"), &common()).unwrap();

    assert_eq!(page.total, 2);
    assert_eq!(
        page.sessions
            .iter()
            .map(|row| row.handle.as_str())
            .collect::<Vec<_>>(),
        ["new", "old"]
    );
}

#[test]
fn find_reuses_fuzzy_fields_and_only_biases_current_workspace() {
    let rows = vec![
        row(
            "other",
            "ultra",
            "Investigate buzz transport",
            SessionState::Offline,
            90,
        ),
        row(
            "local",
            "mosaico",
            "Investigate buzz transport",
            SessionState::Offline,
            80,
        ),
    ];

    let page = query(
        rows,
        Mode::Find {
            query: "buzz",
            current_workspace: Some("mosaico"),
        },
        None,
        &common(),
    )
    .unwrap();

    assert_eq!(page.sessions[0].handle, "local");
    assert_eq!(page.sessions[1].handle, "other");
}

#[test]
fn filters_and_pagination_report_the_full_match_count() {
    let mut args = common();
    args.state = Some("offline".into());
    args.resumable = true;
    args.since = Some("50".into());
    args.limit = 1;
    args.offset = 1;
    let mut rows = vec![
        row("one", "mosaico", "one", SessionState::Offline, 90),
        row("two", "mosaico", "two", SessionState::Offline, 80),
        row("old", "mosaico", "old", SessionState::Offline, 20),
        row("live", "mosaico", "live", SessionState::Idle, 95),
    ];
    rows[0].resumable = true;
    rows[1].resumable = true;
    rows[2].resumable = true;
    rows[3].resumable = true;

    let page = query(rows, Mode::List, None, &args).unwrap();

    assert_eq!(page.total, 2);
    assert_eq!(page.sessions[0].handle, "two");
    assert!(!page.has_more());
    assert_eq!(page.next_offset(), None);
}

#[test]
fn open_turn_is_included_in_rough_busy_hint_and_json() {
    let mut working = row(
        "busy-codex",
        "mosaico",
        "Long task",
        SessionState::Working,
        100,
    );
    working.busy_seconds = 60;
    working.turn_started_at = 80;
    working.turn_count = 3;
    let page = Page {
        sessions: vec![working],
        total: 2,
        limit: 1,
        offset: 0,
        workspace: Some("mosaico".into()),
    };

    let text = render::text(&page, 100);
    assert!(text.contains("busy ~1m"));
    assert!(text.contains("open: mosaico busy-codex"));
    assert!(text.contains("continue with --offset 1"));

    let value: serde_json::Value =
        serde_json::from_str(&render::json(&page, 100).unwrap()).unwrap();
    assert_eq!(value["sessions"][0]["busy_seconds"], 80);
    assert_eq!(value["sessions"][0]["open_command"], "mosaico busy-codex");
    assert_eq!(value["page"]["has_more"], true);
    assert_eq!(value["page"]["next_offset"], 1);
}

#[test]
fn invalid_state_and_since_are_rejected() {
    assert!(parse_state("busy").is_err());
    assert!(parse_since("last-week").is_err());
    assert!(parse_since("2h").is_ok());
    assert_eq!(parse_since("1700000000").unwrap(), 1_700_000_000);
}
