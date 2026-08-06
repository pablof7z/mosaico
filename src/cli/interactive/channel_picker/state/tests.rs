use super::*;

fn node(path: &str, children: Vec<ChannelNode>) -> ChannelNode {
    ChannelNode {
        path: path.into(),
        about: String::new(),
        agents: None,
        last_activity: None,
        children,
    }
}

#[test]
fn flatten_respects_expansion() {
    let forest = vec![node(
        "#root",
        vec![node("#root/a", vec![node("#root/a/b", vec![])])],
    )];
    let mut expanded = BTreeSet::new();
    expanded.insert("#root".into());
    let mut rows = Vec::new();
    flatten(&forest, 0, &expanded, &mut rows);
    assert_eq!(
        rows.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
        vec!["#root", "#root/a"]
    );
    expanded.insert("#root/a".into());
    rows.clear();
    flatten(&forest, 0, &expanded, &mut rows);
    assert_eq!(rows.len(), 3);
}

#[test]
fn refuses_delete_when_children_present() {
    let forest = vec![node("#root", vec![node("#root/a", vec![])])];
    let mut state = PickerState::new(forest);
    // Root still has a child → refuse.
    assert!(state
        .handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE), 10)
        .is_none());
    assert!(state.notice().unwrap().contains("child"));
    assert!(state.pending().is_none());
}

#[test]
fn allows_delete_confirm_for_empty_workspace_root() {
    let forest = vec![node("#root", vec![])];
    let mut state = PickerState::new(forest);
    assert!(state
        .handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE), 10)
        .is_none());
    assert!(matches!(
        state.pending(),
        Some(Pending::ConfirmDelete { path }) if path == "#root"
    ));
}

#[test]
fn allows_delete_confirm_for_leaf_child() {
    let forest = vec![node("#root", vec![node("#root/a", vec![])])];
    let mut state = PickerState::new(forest);
    state.focus_path("#root/a");
    assert!(state
        .handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE), 10)
        .is_none());
    assert!(matches!(
        state.pending(),
        Some(Pending::ConfirmDelete { path }) if path == "#root/a"
    ));
}
