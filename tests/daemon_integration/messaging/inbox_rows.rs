use tenex_edge::state::{InboxRow, Store};

pub(super) fn receiver_inbox_rows(store: &Store, receiver_canon: &str) -> Vec<InboxRow> {
    let mut rows = store.peek_pending_for_session(receiver_canon).unwrap();
    rows.extend(
        store
            .recently_delivered_for_session(receiver_canon, 0)
            .unwrap(),
    );
    rows
}
