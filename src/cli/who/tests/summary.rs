//! The other-workspaces summary counts presence by heartbeat, the same fact as
//! agent counts: neither a closing offline status nor local lifecycle alone.

use super::*;

fn two_roots(store: &Store) {
    store.install_test_group_delivery(TestGroupDelivery::new([
        TestGroup::new("proj").metadata("proj", "", "", 1),
        TestGroup::new("other").metadata("other", "", "", 1),
    ]));
}

fn in_other(status: Status) -> Status {
    Status {
        channel_h: "other".into(),
        ..status
    }
}

fn other_agents(store: &Store) -> Vec<String> {
    load_who_snapshot(store, Some("proj"), 100, "laptop")
        .unwrap()
        .other_roots
        .into_iter()
        .find(|root| root.root.contains("other"))
        .map(|root| root.agents)
        .unwrap_or_default()
}

#[test]
fn a_peer_closing_offline_elsewhere_is_not_counted() {
    let store = Store::open_memory().unwrap();
    two_roots(&store);
    let (live_profile, live) = peer("pk-live", "live-peer", "laptop", "Working", true, 90);
    let (gone_profile, gone) = peer("pk-gone", "gone-peer", "laptop", "Done", false, 90);
    install_peers(
        &store,
        [
            (live_profile, in_other(live)),
            (
                gone_profile,
                in_other(Status {
                    state: crate::session_state::SessionState::Offline,
                    ..gone
                }),
            ),
        ],
    );

    assert_eq!(other_agents(&store), vec!["live-peer".to_string()]);
}

#[test]
fn a_local_session_elsewhere_is_counted_only_with_a_heartbeat() {
    let store = Store::open_memory().unwrap();
    two_roots(&store);
    register_local_in(&store, "coder", "pk-local", "other", "ext", 90);
    assert!(
        other_agents(&store).is_empty(),
        "running alone is not presence"
    );

    let (profile, status) = peer("pk-local", "coder", "laptop", "Coding", true, 90);
    install_peers(&store, [(profile, in_other(status))]);
    assert_eq!(other_agents(&store).len(), 1);
}
