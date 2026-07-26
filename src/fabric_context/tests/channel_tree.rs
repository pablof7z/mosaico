use super::*;

#[test]
fn session_view_has_self_and_chatter_human_view_does_not() {
    let store = seed_store();
    let rec = session(&store);
    chat(&store, "m1", "root", 900, "post join context", "[]");
    publish_idle_status(&store, OTHER_PK, "reviewer", "Reviewing");

    let agent = render_fabric_context(&store, input(Some(&rec), "root", 0, 1_000, false))
        .expect("session view should render");
    assert!(
        agent.contains("<self name=\"@coder\" host=\"laptop\" headless=\"off\" workspace=\"root\"")
    );
    assert!(agent.contains("<chatter>"));
    assert!(
        agent.contains("<message from=\"@reviewer\" id=\"m1\" age=\"1 min ago\">post join context"),
        "every agent-visible message must expose its reaction/reply id: {agent}"
    );
    assert!(agent.contains("<channels>"));
    assert!(!agent.contains("<workspace"));
    assert!(agent.contains("<channel name=\"/root/task\""));
    assert!(agent.contains("<channel name=\"/root\""));
    for path in ["/root", "/root/task"] {
        assert!(
            !agent.contains(&format!("<channel name=\"{path}\" id=\"")),
            "{agent}"
        );
    }
    let channels = agent.find("<channels>").unwrap();
    let root_channel = agent.find("<channel name=\"/root\"").unwrap();
    let members = agent.find("<members>").unwrap();
    assert!(
        channels < root_channel && root_channel < members,
        "got: {agent}"
    );
    assert!(!agent.contains("<subchannels>"));
    assert!(!agent.contains("<channels-not-joined>"));

    let human = render_fabric_context(&store, input(None, "root", 0, 1_000, true))
        .expect("human who should render");
    assert!(human.contains("<mosaico>"));
    assert!(!human.contains("Session: @"));
    assert!(!human.contains("<chatter>"));
}

#[test]
fn cursor_delta_only_renders_changed_joined_channel() {
    let store = seed_store();
    let rec = session(&store);
    chat(&store, "old-root", "root", 100, "old root message", "[]");
    chat(&store, "new-task", TASK_H, 220, "new task message", "[]");

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false))
        .expect("changed task channel should render");
    assert!(text.contains("name=\"/root/task\""));
    assert!(text.contains("new task message"));
    assert!(!text.contains("name=\"#main\""));
    assert!(!text.contains("old root message"));
}

#[test]
fn presence_delta_does_not_repeat_unchanged_descendants() {
    let store = seed_store();
    let rec = session(&store);
    store
        .upsert_status(&Status {
            pubkey: OTHER_PK.into(),
            channel_h: "root".into(),
            slug: "amber-reviewer".into(),
            title: "Reviewing".into(),
            activity: "checking tests".into(),
            workspace: "root".into(),
            branch: String::new(),
            state: crate::session_state::SessionState::Working,
            state_since: 250,
            last_seen: 250,
            updated_at: 250,
            expiration: 500,
        })
        .unwrap();

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false))
        .expect("presence delta should render");
    assert!(text.contains("<members>"));
    assert!(!text.contains("<recent-presence>"));
    assert!(text.contains("name=\"@amber-reviewer\""));
    assert!(text.contains("status=\"checking tests\""));
    assert!(
        !text.contains("/root/task"),
        "unchanged descendants must not ride along with presence deltas: {text}"
    );
}

#[test]
fn changed_descendant_metadata_renders_once_with_its_canonical_ref() {
    let store = seed_store();
    let rec = session(&store);
    store
        .upsert_channel(TASK_H, "task", "Updated task room", "root", 250)
        .unwrap();

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false))
        .expect("changed descendant should render");
    assert!(!text.contains("<workspace"));
    assert!(
        text.contains("<channel name=\"/root/task\" about=\"Updated task room\" agents=\"2\" />")
    );
    assert_eq!(text.matches("name=\"/root/task\"").count(), 1, "{text}");
    assert!(!text.contains(" id=\""), "{text}");
    assert!(!text.contains("<subchannels>"), "{text}");
    assert!(!text.contains("<channels-not-joined>"), "{text}");

    let captured = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    assert_eq!(
        render_view_text(&assemble::assemble_view(&captured, 200, 300)),
        text
    );
}

#[test]
fn full_snapshot_nests_multilevel_channels_by_slash_reference() {
    let store = seed_store();
    store
        .upsert_channel("leaf-h", "leaf", "Leaf room", TASK_H, 2)
        .unwrap();
    let rec = session(&store);
    let captured = capture_inputs(&store, &input(Some(&rec), "root", 0, 300, false)).unwrap();
    assert!(captured.meta.joined_channels.contains("root"));

    let text = render_fabric_context(&store, input(Some(&rec), "root", 0, 300, false))
        .expect("full descendant tree should render");
    let task = text.find("name=\"/root/task\"").expect("task path");
    let leaf = text.find("name=\"/root/task/leaf\"").expect("leaf path");
    assert!(task < leaf, "child must follow its parent: {text}");
    assert_eq!(text.matches("name=\"/root/task\"").count(), 1, "{text}");

    let captured = capture_inputs(&store, &input(Some(&rec), "root", 0, 300, false)).unwrap();
    assert_eq!(
        render_view_text(&assemble::assemble_view(&captured, 0, 300)),
        text
    );

    let human =
        render_fabric_context_human(&store, input(Some(&rec), "root", 0, 300, false), false)
            .expect("valid channel ancestry")
            .expect("human tree should render");
    assert!(human.contains("/root/task"), "{human}");
    assert!(human.contains("/root/task/leaf"), "{human}");
}

#[test]
fn joined_root_expands_every_descendant_even_when_the_session_did_not_join_them() {
    let store = seed_store();
    store
        .upsert_channel("other-h", "other", "Unjoined sibling", "root", 2)
        .unwrap();
    store
        .upsert_channel("deep-h", "deep", "Unjoined grandchild", "other-h", 3)
        .unwrap();
    let rec = session(&store);
    let captured = capture_inputs(&store, &input(Some(&rec), "root", 0, 300, false)).unwrap();
    assert!(
        captured
            .meta
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.channels)
            .any(|channel| channel.reference == "/root/other/deep"),
        "channels={:?}; captured={:?}",
        store.list_channels().unwrap(),
        captured.meta.workspaces,
    );

    let text = render_fabric_context(&store, input(Some(&rec), "root", 0, 300, false))
        .expect("the joined root's complete hierarchy should render");
    assert!(text.contains("name=\"/root/other\""), "{text}");
    assert!(text.contains("name=\"/root/other/deep\""), "{text}");
    assert!(
        text.find("name=\"/root/other\"").unwrap()
            < text.find("name=\"/root/other/deep\"").unwrap(),
        "{text}"
    );
}
