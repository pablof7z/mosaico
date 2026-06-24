use super::*;

#[test]
fn row_project_for_tabs_prefers_work_root_over_routing_scope() {
    let row = serde_json::json!({
        "project": "session-deadbeef",
        "work_root": "tenex-edge",
    });

    assert_eq!(row_project_for_tabs(&row), "tenex-edge");
}

#[test]
fn project_tabs_do_not_show_session_room_after_normalization() {
    let session_room = serde_json::json!({
        "project": "session-deadbeef",
        "work_root": "tenex-edge",
    });
    let data = TuiData {
        live: vec![LiveRow {
            slug: "codex".to_string(),
            host: "laptop".to_string(),
            project: row_project_for_tabs(&session_room),
            session_id: "sess-a".to_string(),
            session_codename: "alpha0001".to_string(),
            status: "working".to_string(),
            attachable: true,
        }],
        spawnable: vec![],
        resumable: vec![],
    };

    let tabs = compute_project_tabs(&data);
    assert_eq!(tabs.visible, vec!["tenex-edge".to_string()]);
    assert!(!tabs.visible.contains(&"session-deadbeef".to_string()));
}
