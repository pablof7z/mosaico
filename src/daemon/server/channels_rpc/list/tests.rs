use super::projection::{build, ListMode};
use super::*;
use crate::state::{RecordMessage, Store};

fn channel(store: &Store, id: &str, name: &str, about: &str, parent: &str, at: u64) {
    store.upsert_channel(id, name, about, parent, at).unwrap();
}

fn topology() -> Store {
    let store = Store::open_memory().unwrap();
    channel(&store, "own", "general", "Primary workspace", "", 1);
    channel(&store, "alpha-h", "alpha", "Alpha work", "own", 2);
    channel(&store, "deep-h", "deep", "Deep work", "alpha-h", 3);
    channel(&store, "joined", "general", "Peer workspace", "", 1);
    channel(&store, "review-h", "review", "Review work", "joined", 2);
    channel(&store, "dev2", "general", "Other workspace", "", 1);
    channel(&store, "one-h", "one", "First", "dev2", 2);
    channel(&store, "two-h", "two", "Second", "one-h", 3);
    store
}

#[test]
fn caller_view_expands_own_and_joined_but_compacts_other_roots() {
    let store = topology();
    let view = build(
        &store,
        ListMode::Caller {
            own: "own".into(),
            joined: BTreeSet::from(["own".into(), "joined".into()]),
        },
        100,
        "",
    )
    .unwrap();

    assert_eq!(
        view.sections
            .iter()
            .map(|section| section.kind)
            .collect::<Vec<_>>(),
        ["own", "joined", "other"]
    );
    let own = &view.sections[0].channels[0];
    assert_eq!(own.path, "/own");
    assert_eq!(own.about, "Primary workspace");
    assert_eq!(own.children[0].path, "/own/alpha");
    assert_eq!(own.children[0].children[0].path, "/own/alpha/deep");

    let joined = &view.sections[1].channels[0];
    assert_eq!(joined.children[0].path, "/joined/review");

    let other = &view.sections[2].channels[0];
    assert_eq!(other.path, "/dev2");
    assert_eq!(other.subchannels, Some(2));
    assert!(other.children.is_empty());
}

#[test]
fn launch_workspace_is_compact_when_the_session_joined_no_channel_under_it() {
    let store = topology();
    let view = build(
        &store,
        ListMode::Caller {
            own: "own".into(),
            joined: BTreeSet::from(["joined".into()]),
        },
        100,
        "",
    )
    .unwrap();

    let own = &view.sections[0].channels[0];
    assert_eq!(own.path, "/own");
    assert_eq!(own.subchannels, Some(2));
    assert!(own.children.is_empty());
    assert_eq!(
        view.sections[1].channels[0].children[0].path,
        "/joined/review"
    );
}

#[test]
fn recursive_view_expands_unjoined_roots_without_opaque_ids() {
    let store = topology();
    let view = build(&store, ListMode::Recursive, 100, "").unwrap();
    let json = serde_json::to_string(&view).unwrap();

    let dev2 = view.sections[0]
        .channels
        .iter()
        .find(|root| root.path == "/dev2")
        .unwrap();
    assert_eq!(dev2.children[0].path, "/dev2/one");
    assert_eq!(dev2.children[0].children[0].path, "/dev2/one/two");
    assert!(!json.contains("one-h"));
    assert!(!json.contains("child_h"));
}

#[test]
fn counts_named_agents_only_when_both_roster_snapshots_are_hydrated() {
    let store = topology();
    store
        .upsert_profile_with_agent_slug(
            "agent-pk", "reviewer", "reviewer", "reviewer", "laptop", false, 1,
        )
        .unwrap();
    store
        .upsert_profile("human-pk", "Pablo", "pablo", "laptop", false, 1)
        .unwrap();
    store
        .upsert_profile("backend-pk", "backend", "backend", "laptop", true, 1)
        .unwrap();
    store
        .replace_channel_members(
            "own",
            &["agent-pk".into(), "human-pk".into(), "backend-pk".into()],
            2,
        )
        .unwrap();
    store
        .replace_channel_admins("own", &["backend-pk".into(), "unknown-admin".into()], 2)
        .unwrap();
    // Only one of the two relay-authored sets has arrived for alpha.
    store
        .replace_channel_members("alpha-h", &["agent-pk".into()], 2)
        .unwrap();
    store
        .record_message(&RecordMessage {
            message_id: "msg".into(),
            thread_id: "msg".into(),
            channel_h: "own".into(),
            author_pubkey: "agent-pk".into(),
            body: "hello".into(),
            created_at: 40,
            direction: "inbound".into(),
            sync_state: "accepted".into(),
            native_event_id: Some("event".into()),
            error: None,
        })
        .unwrap();

    let view = build(&store, ListMode::Workspace("own".into()), 100, "backend-pk").unwrap();
    let root = &view.sections[0].channels[0];
    assert_eq!(root.agents, Some(1));
    assert_eq!(root.last_activity.as_deref(), Some("1 min ago"));
    assert_eq!(root.children[0].agents, None);

    let json = serde_json::to_value(view).unwrap();
    assert_eq!(json["sections"][0]["channels"][0]["agents"], 1);
    assert!(json["sections"][0]["channels"][0]["children"][0]
        .get("agents")
        .is_none());
}

#[test]
fn compact_empty_root_has_no_zero_subchannel_suffix() {
    let store = Store::open_memory().unwrap();
    channel(&store, "solo", "general", "No children", "", 1);
    let view = build(&store, ListMode::All, 10, "").unwrap();

    let solo = &view.sections[0].channels[0];
    assert_eq!(solo.path, "/solo");
    assert_eq!(solo.subchannels, None);
}

#[test]
fn hydrated_roster_with_an_unclassified_identity_omits_the_agent_count() {
    let store = topology();
    store
        .replace_channel_members("own", &["unknown-pk".into()], 2)
        .unwrap();
    store.replace_channel_admins("own", &[], 2).unwrap();

    let view = build(&store, ListMode::Workspace("own".into()), 10, "").unwrap();
    assert_eq!(view.sections[0].channels[0].agents, None);
}

#[test]
fn explicit_workspace_keeps_a_cold_registered_root_visible() {
    let store = Store::open_memory().unwrap();
    let view = build(&store, ListMode::Workspace("cold".into()), 10, "").unwrap();

    let root = &view.sections[0].channels[0];
    assert_eq!(root.path, "/cold");
    assert_eq!(root.about, "");
    assert_eq!(root.agents, None);
}
