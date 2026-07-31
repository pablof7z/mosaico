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
    // Move to the child leaf that still has no children — first expand, then down.
    state.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 10);
    let key = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE);
    // Child under root with no further children should enter confirm.
    // Root itself is selected first: expect root refusal.
    let mut root_state = PickerState::new(vec![node("#root", vec![node("#root/a", vec![])])]);
    assert!(root_state
        .handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE), 10)
        .is_none());
    assert!(root_state.notice().unwrap().contains("workspace root"));

    // Focus the leaf and confirm delete is offered.
    let forest = vec![node("#root", vec![node("#root/a", vec![])])];
    let mut state = PickerState::new(forest);
    state.focus_path("#root/a");
    assert!(state.handle_key(key, 10).is_none());
    assert!(matches!(
        state.pending(),
        Some(Pending::ConfirmDelete { path }) if path == "#root/a"
    ));
}
