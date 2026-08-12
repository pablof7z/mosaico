use super::*;

#[test]
fn literal_id_selectors_are_rejected_in_every_form() {
    let store = Store::open_memory().unwrap();
    channels(
        &store,
        &[("h-root", "proj", ""), ("h-plan", "planning", "h-root")],
    );
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "h-plan"),
        ChannelResolution::NotFound
    ));
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "@h-plan"),
        ChannelResolution::NotFound
    ));
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "@nonexistent"),
        ChannelResolution::NotFound
    ));
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "#h-root/planning"),
        ChannelResolution::Unique(ref id) if id == "h-plan"
    ));
}

#[test]
fn unnamed_internal_ancestry_cannot_be_bypassed_with_an_id_selector() {
    let store = Store::open_memory().unwrap();
    channels(
        &store,
        &[
            ("h-root", "workspace", ""),
            ("session-room", "session-room", "h-root"),
            ("abcd1234", "editable", "session-room"),
        ],
    );

    assert!(matches!(
        resolve_absolute_channel_ref(&store, "#h-root/editable"),
        ChannelResolution::NotFound
    ));
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "@abcd"),
        ChannelResolution::NotFound
    ));
}

#[test]
fn nested_sender_explicit_channel_refs_resolve_from_root_channel() {
    let store = Store::open_memory().unwrap();
    channels(
        &store,
        &[
            ("h-root", "proj", ""),
            ("h-epic", "epic", "h-root"),
            ("h-plan", "planning", "h-epic"),
            ("h-leaf", "leaf", "h-plan"),
            ("h-review", "review", "h-epic"),
        ],
    );

    let root = root_channel(&store, "h-leaf").unwrap();
    assert_eq!(root, "h-root");
    assert!(matches!(
        resolve_absolute_channel_ref(&store, "#h-root/epic/review"),
        ChannelResolution::Unique(ref id) if id == "h-review"
    ));
}

#[test]
fn describe_missing_channel_lists_the_workspace_and_sibling_workspaces() {
    let store = Store::open_memory().unwrap();
    channels(
        &store,
        &[
            ("workspace", "workspace", ""),
            ("h-alpha", "alpha", "workspace"),
            ("test", "test", ""),
            ("h-foo", "foo", "test"),
            ("hello", "hello", ""),
        ],
    );

    let message = describe_missing_channel(&store, "#workspace/test/hello");
    assert!(message.contains("no channel matching"));
    assert!(message.contains("Channels in #workspace:"));
    assert!(message.contains("#workspace/alpha"));
    assert!(message.contains("#test is also a separate workspace"));
    assert!(message.contains("#test/foo"));
    assert!(message.contains("#hello is also a separate workspace"));
}

#[test]
fn describe_missing_channel_lists_known_workspaces_when_root_missing() {
    let store = Store::open_memory().unwrap();
    channels(&store, &[("nmp", "general", "")]);
    let message = describe_missing_channel(&store, "#nonexistent/child");
    assert!(message.contains("No workspace named \"nonexistent\""));
    assert!(message.contains("#nmp"));
}
