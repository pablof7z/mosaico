use super::*;

fn project_session() -> SessionChoice {
    let mut session = session_choice("juno", "mosaico work", false);
    session.row.workspaces = vec![
        crate::cli::interactive::session_picker::data::WorkspaceGroup {
            id: "mosaico-id".into(),
            name: "mosaico".into(),
            path: Some("/repo/mosaico".into()),
            ..Default::default()
        },
    ];
    session
}

#[test]
fn manual_project_filter_toggles_without_affecting_the_agent_tab() {
    let mut picker = state(vec![
        HomeChoice::Session(project_session()),
        session("skills", "skills work", false),
        agent("codex", AgentKind::Generic),
    ]);
    picker.handle_key(ctrl(KeyCode::Char('p')), 10);
    picker.handle_key(key(KeyCode::Down), 10);
    picker.handle_key(key(KeyCode::Enter), 10);

    assert_eq!(picker.project_filter.as_deref(), Some("mosaico-id"));
    assert_eq!(picker.visible, vec![0]);
    picker.handle_key(key(KeyCode::Tab), 10);
    assert_eq!(picker.visible, vec![0, 1]);
    picker.handle_key(key(KeyCode::Tab), 10);
    assert_eq!(picker.visible, vec![0]);
    picker.handle_key(key(KeyCode::Right), 10);
    assert_eq!(picker.visible, vec![2]);
}

#[test]
fn current_project_filter_is_enabled_by_default_and_tab_toggles_it() {
    let mut picker = state(vec![
        HomeChoice::Session(project_session()),
        session("skills", "skills work", false),
    ])
    .with_project_filter(Some("mosaico-id".into()));

    assert_eq!(picker.project_filter.as_deref(), Some("mosaico-id"));
    assert_eq!(picker.visible, vec![0]);
    picker.handle_key(key(KeyCode::Tab), 10);
    assert_eq!(picker.project_filter, None);
    assert_eq!(picker.visible, vec![0, 1]);
    picker.handle_key(key(KeyCode::Tab), 10);
    assert_eq!(picker.project_filter.as_deref(), Some("mosaico-id"));
    assert_eq!(picker.visible, vec![0]);
}
