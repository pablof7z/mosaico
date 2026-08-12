use super::projection::{build, ListMode};
use super::*;
use crate::state::{Profile, RelayEvent, Store, TestGroup, TestGroupDelivery, TestRelayDelivery};

fn topology() -> Store {
    topology_with_records(None, None, None)
}

fn topology_with_records(
    own_admins: Option<Vec<String>>,
    own_members: Option<Vec<String>>,
    acquiring_alpha_members: Option<Vec<String>>,
) -> Store {
    let store = Store::open_memory().unwrap();
    let mut own = TestGroup::new("own").metadata("general", "Primary workspace", "", 1);
    if let Some(admins) = own_admins {
        own = own.admins(admins);
    }
    if let Some(members) = own_members {
        own = own.members(members);
    }
    let mut alpha = TestGroup::new("alpha-h").metadata("alpha", "Alpha work", "own", 2);
    if let Some(members) = acquiring_alpha_members {
        alpha = alpha
            .members(members)
            .availability(nmp::nip29::GroupAvailability::Acquiring);
    }
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([
        own,
        alpha,
        TestGroup::new("deep-h").metadata("deep", "Deep work", "alpha-h", 3),
        TestGroup::new("joined").metadata("general", "Peer workspace", "", 1),
        TestGroup::new("review-h").metadata("review", "Review work", "joined", 2),
        TestGroup::new("dev2").metadata("general", "Other workspace", "", 1),
        TestGroup::new("one-h").metadata("one", "First", "dev2", 2),
        TestGroup::new("two-h").metadata("two", "Second", "one-h", 3),
    ]));
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
    assert_eq!(own.path, "#own");
    assert_eq!(own.about, "Primary workspace");
    assert_eq!(own.children[0].path, "#own/alpha");
    assert_eq!(own.children[0].children[0].path, "#own/alpha/deep");

    let joined = &view.sections[1].channels[0];
    assert_eq!(joined.children[0].path, "#joined/review");

    let other = &view.sections[2].channels[0];
    assert_eq!(other.path, "#dev2");
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
    assert_eq!(own.path, "#own");
    assert_eq!(own.subchannels, Some(2));
    assert!(own.children.is_empty());
    assert_eq!(
        view.sections[1].channels[0].children[0].path,
        "#joined/review"
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
        .find(|root| root.path == "#dev2")
        .unwrap();
    assert_eq!(dev2.children[0].path, "#dev2/one");
    assert_eq!(dev2.children[0].children[0].path, "#dev2/one/two");
    assert!(!json.contains("one-h"));
    assert!(!json.contains("child_h"));
}

#[test]
fn counts_named_agents_only_when_group_state_is_available() {
    let store = topology_with_records(
        Some(vec!["backend-pk".into(), "unknown-admin".into()]),
        Some(vec![
            "agent-pk".into(),
            "human-pk".into(),
            "backend-pk".into(),
        ]),
        Some(vec!["agent-pk".into()]),
    );
    // NMP reports the child group as still acquiring, so its member count is
    // not yet a complete product fact.
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles([
                Profile {
                    pubkey: "agent-pk".into(),
                    name: "reviewer".into(),
                    slug: "reviewer".into(),
                    agent_slug: "reviewer".into(),
                    host: "laptop".into(),
                    is_backend: false,
                    agents: Vec::new(),
                    workspaces: Vec::new(),
                    updated_at: 1,
                },
                Profile {
                    pubkey: "human-pk".into(),
                    name: "Pablo".into(),
                    slug: "pablo".into(),
                    agent_slug: String::new(),
                    host: "laptop".into(),
                    is_backend: false,
                    agents: Vec::new(),
                    workspaces: Vec::new(),
                    updated_at: 1,
                },
                Profile {
                    pubkey: "backend-pk".into(),
                    name: "backend".into(),
                    slug: "backend".into(),
                    agent_slug: "backend".into(),
                    host: "laptop".into(),
                    is_backend: true,
                    agents: Vec::new(),
                    workspaces: Vec::new(),
                    updated_at: 1,
                },
            ])
            .events([RelayEvent {
                id: "msg".into(),
                kind: 9,
                pubkey: "agent-pk".into(),
                created_at: 40,
                channel_h: "own".into(),
                d_tag: String::new(),
                content: "hello".into(),
                tags_json: "[]".into(),
            }]),
    );

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
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([
        TestGroup::new("solo").metadata("general", "No children", "", 1)
    ]));
    let view = build(&store, ListMode::All, 10, "").unwrap();

    let solo = &view.sections[0].channels[0];
    assert_eq!(solo.path, "#solo");
    assert_eq!(solo.subchannels, None);
}

#[test]
fn hydrated_roster_with_an_unclassified_identity_omits_the_agent_count() {
    let store = topology_with_records(Some(Vec::new()), Some(vec!["unknown-pk".into()]), None);

    let view = build(&store, ListMode::Workspace("own".into()), 10, "").unwrap();
    assert_eq!(view.sections[0].channels[0].agents, None);
}

#[test]
fn explicit_workspace_keeps_a_cold_registered_root_visible() {
    let store = Store::open_memory().unwrap();
    let view = build(&store, ListMode::Workspace("cold".into()), 10, "").unwrap();

    let root = &view.sections[0].channels[0];
    assert_eq!(root.path, "#cold");
    assert_eq!(root.about, "");
    assert_eq!(root.agents, None);
}
