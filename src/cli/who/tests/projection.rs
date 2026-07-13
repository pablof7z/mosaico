use super::*;

#[test]
fn who_snapshot_exposes_work_root_for_session_room_rows() {
    let store = Store::open_memory().unwrap();
    store.upsert_channel("proj", "proj", "", "", 1_000).unwrap();
    store
        .upsert_channel("session-room", "session-room", "", "proj", 1_000)
        .unwrap();
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
    store.upsert_channel("root", "root", "", "", 1_000).unwrap();
    store
        .upsert_channel("task", "Task", "", "root", 1_000)
        .unwrap();
    store
        .upsert_channel("leaf", "Leaf", "", "task", 1_000)
        .unwrap();
    register_local_in(&store, "coder", "pk-coder", "leaf", "sid-coder", 1_000);

    let snapshot = load_who_snapshot(&store, Some("root"), 1_000, "laptop").unwrap();
    let row = snapshot.rows.first().expect("nested channel row");
    assert_eq!(row.channel, "leaf");
    assert_eq!(row.work_root, "root");
}
