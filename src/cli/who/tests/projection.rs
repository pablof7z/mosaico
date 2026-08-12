use super::*;

#[test]
fn who_snapshot_exposes_work_root_for_session_room_rows() {
    let store = Store::open_memory().unwrap();
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([
        TestGroup::new("proj").metadata("proj", "", "", 1_000),
        TestGroup::new("session-room").metadata("session-room", "", "proj", 1_000),
    ]));
    register_local_in(
        &store,
        "coder",
        "pk-coder",
        "session-room",
        "sid-coder",
        1_000,
    );

    let snapshot = load_who_snapshot(&store, Some("session-room"), 1_000, "laptop").unwrap();
    let row = snapshot.rows.first().expect("session-room row");
    assert_eq!(row.channel, "session-room");
    assert_eq!(row.work_root, "proj");
}

#[test]
fn who_root_snapshot_includes_nested_channel_sessions() {
    let store = Store::open_memory().unwrap();
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([
        TestGroup::new("root").metadata("root", "", "", 1_000),
        TestGroup::new("task").metadata("Task", "", "root", 1_000),
        TestGroup::new("leaf").metadata("Leaf", "", "task", 1_000),
    ]));
    register_local_in(&store, "coder", "pk-coder", "leaf", "sid-coder", 1_000);

    let snapshot = load_who_snapshot(&store, Some("root"), 1_000, "laptop").unwrap();
    let row = snapshot.rows.first().expect("nested channel row");
    assert_eq!(row.channel, "leaf");
    assert_eq!(row.work_root, "root");
}
