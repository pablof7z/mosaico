#[test]
fn outbox_failed_publish_stays_pending() {
    let store = crate::state::Store::open_memory().unwrap();
    let id = store.enqueue_outbox("{\"kind\":30315}", 1).unwrap();

    store
        .mark_failed(id, "relay rejected event: blocked")
        .unwrap();

    let pending = store.peek_outbox(10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].local_id, id);
    assert_eq!(pending[0].retries, 1);
    assert_eq!(
        pending[0].last_error.as_deref(),
        Some("relay rejected event: blocked")
    );
}
