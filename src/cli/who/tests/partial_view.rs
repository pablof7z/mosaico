//! A partially delivered group view (a child whose ancestry has not arrived,
//! as while a restarted daemon rehydrates) leaves that channel "not yet
//! known": it never fails the whole `who` snapshot.

use super::*;

#[test]
fn a_channel_with_incomplete_ancestry_is_skipped_not_fatal() {
    let store = Store::open_memory().unwrap();
    store.install_test_group_delivery(TestGroupDelivery::new([
        TestGroup::new("proj").metadata("proj", "", "", 1),
        TestGroup::new("child").metadata("child", "", "missing-mid", 1),
    ]));
    register_local_in(&store, "coder", "pk-root", "proj", "ext-root", 100);
    register_local_in(&store, "helper", "pk-child", "child", "ext-child", 100);
    store.grant_session_route("pk-child", "child", 100).unwrap();
    let (profile, status) = peer("pk-peer", "peer", "laptop", "Working", true, 100);
    install_peers(
        &store,
        [(
            profile,
            Status {
                channel_h: "child".into(),
                ..status
            },
        )],
    );

    load_who_snapshot(&store, None, 100, "laptop").expect("a partial view must not fail who");
    let snapshot = load_who_snapshot(&store, Some("proj"), 100, "laptop")
        .expect("a partial view must not fail who");
    let channels = snapshot
        .rows
        .iter()
        .map(|row| row.channel.as_str())
        .collect::<Vec<_>>();
    assert!(channels.contains(&"proj"), "rows: {channels:?}");
    assert!(!channels.contains(&"child"), "rows: {channels:?}");
    // Leaving the channel out is visible, not silent.
    assert_eq!(snapshot.unresolved_channels, ["child"]);
    let value = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(
        super::super::unresolved_note(&value).as_deref(),
        Some(
            "note: 1 channel(s) not shown: their parent channels have not \
             been delivered by the relay yet"
        )
    );
}
