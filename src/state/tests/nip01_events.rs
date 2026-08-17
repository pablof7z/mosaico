use super::super::*;

fn event(id: &str, kind: u32, created_at: u64, d_tag: &str) -> RelayEvent {
    RelayEvent {
        id: id.into(),
        kind,
        pubkey: "pk".into(),
        created_at,
        channel_h: "h1".into(),
        d_tag: d_tag.into(),
        content: id.into(),
        tags_json: "[]".into(),
    }
}

#[test]
fn cache_never_picks_a_second_replaceable_winner() {
    let store = Store::open_memory().unwrap();
    let first = event("first", 30078, 100, "coordinate");
    let second = event("second", 30078, 200, "coordinate");

    assert!(store.insert_event(&first).unwrap());
    assert!(store.insert_event(&second).unwrap());
    assert!(store.get_event("first").unwrap().is_some());
    assert!(store.get_event("second").unwrap().is_some());

    // NMP's exact Removed transition, not a SQLite comparator, selects what
    // leaves the cache.
    assert!(store.retract_event("first").unwrap());
    assert!(store.get_event("first").unwrap().is_none());
    assert!(store.get_event("second").unwrap().is_some());
}

#[test]
fn same_second_winner_is_whichever_row_nmp_adds_after_its_removal() {
    let store = Store::open_memory().unwrap();
    let high = event("ffff", 100, 100, "");
    let low = event("0000", 100, 100, "");

    assert!(store.insert_event(&high).unwrap());
    assert!(store.retract_event(&high.id).unwrap());
    assert!(store.insert_event(&low).unwrap());

    assert!(store.get_event(&high.id).unwrap().is_none());
    assert_eq!(store.get_event(&low.id).unwrap().unwrap().content, "0000");
}

#[test]
fn regular_rows_append_and_exact_redelivery_is_idempotent() {
    let store = Store::open_memory().unwrap();
    let first = event("n1", 1, 1, "");
    let second = event("n2", 1, 1, "");
    assert!(store.insert_event(&first).unwrap());
    assert!(store.insert_event(&second).unwrap());
    assert!(!store.insert_event(&first).unwrap());
    assert_eq!(store.chat_for_channel("h1", 0, 10).unwrap().len(), 2);
}
