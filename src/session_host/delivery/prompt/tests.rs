use super::*;

#[tokio::test]
async fn failed_delivery_leaves_reminder_due_and_success_records_it() {
    let state = DaemonState::new_for_test().await;
    assert!(state.coordination_reminder_due("session-a", 1));

    let failed =
        finish_delivery::<()>(&state, "session-a", Some(1), Err(anyhow::anyhow!("failed")));
    assert!(failed.is_err());
    assert!(state.coordination_reminder_due("session-a", 1));

    finish_delivery(&state, "session-a", Some(1), Ok(())).unwrap();
    assert!(!state.coordination_reminder_due("session-a", 1));
    assert!(!state.coordination_reminder_due("session-a", 8));
    assert!(state.coordination_reminder_due("session-a", 9));
    assert!(state.coordination_reminder_due("session-b", 1));
}
