use crate::state::{Status, Store};

fn status(activity: &str, state: crate::session_state::SessionState, updated_at: u64) -> Status {
    Status {
        pubkey: "pk".into(),
        channel_h: "h1".into(),
        slug: "agent".into(),
        title: "Task title".into(),
        activity: activity.into(),
        workspace: "mosaico".into(),
        branch: "feat/state".into(),
        state,
        state_since: 100,
        last_seen: updated_at,
        updated_at,
        expiration: updated_at + 100,
    }
}

#[test]
fn lease_renewal_refreshes_liveness_without_advancing_delta_clock() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_status(&status(
            "reading",
            crate::session_state::SessionState::Working,
            100,
        ))
        .unwrap();
    store
        .upsert_status(&status(
            "reading",
            crate::session_state::SessionState::Working,
            150,
        ))
        .unwrap();
    let row = store.get_status("pk", "h1").unwrap().unwrap();
    assert_eq!(
        (
            row.state_since,
            row.last_seen,
            row.expiration,
            row.updated_at
        ),
        (100, 150, 250, 100)
    );
}

#[test]
fn semantic_status_change_advances_delta_clock() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_status(&status(
            "reading",
            crate::session_state::SessionState::Working,
            100,
        ))
        .unwrap();
    store
        .upsert_status(&status(
            "writing",
            crate::session_state::SessionState::Working,
            150,
        ))
        .unwrap();
    let row = store.get_status("pk", "h1").unwrap().unwrap();
    assert_eq!((row.activity.as_str(), row.updated_at), ("writing", 150));
    assert_eq!(row.state_since, 100);
}

#[test]
fn replacement_lease_renewal_preserves_delta_clock_and_removes_absent_channels() {
    let store = Store::open_memory().unwrap();
    let initial = status("reading", crate::session_state::SessionState::Working, 100);
    let mut side = initial.clone();
    side.channel_h = "h2".into();
    store
        .replace_status_channels("pk", &[initial.clone(), side], 100)
        .unwrap();

    let renewal = status("reading", crate::session_state::SessionState::Working, 150);
    store
        .replace_status_channels("pk", &[renewal], 150)
        .unwrap();

    let row = store.get_status("pk", "h1").unwrap().unwrap();
    assert_eq!(
        (row.last_seen, row.expiration, row.updated_at),
        (150, 250, 100)
    );
    assert!(store.get_status("pk", "h2").unwrap().is_none());
    let stale = status("reading", crate::session_state::SessionState::Working, 125);
    assert!(!store.replace_status_channels("pk", &[stale], 125).unwrap());
    assert_eq!(
        store.get_status("pk", "h1").unwrap().unwrap().last_seen,
        150
    );
}
