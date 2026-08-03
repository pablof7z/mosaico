use super::super::*;

#[test]
fn nip01_replaceable_replaces_by_kind_pubkey() {
    let s = Store::open_memory().unwrap();
    let mut ev = RelayEvent {
        id: "e1".into(),
        kind: 10002,
        pubkey: "pk".into(),
        created_at: 100,
        channel_h: String::new(),
        d_tag: String::new(),
        content: "old".into(),
        tags_json: "[]".into(),
    };
    assert!(s.insert_event(&ev).unwrap());
    ev.id = "e2".into();
    ev.created_at = 200;
    ev.content = "new".into();
    assert!(s.insert_event(&ev).unwrap());
    assert!(s.get_event("e1").unwrap().is_none());
    assert_eq!(s.get_event("e2").unwrap().unwrap().content, "new");
    // An older event loses the race and is not stored.
    ev.id = "e0".into();
    ev.created_at = 50;
    assert!(!s.insert_event(&ev).unwrap());
    assert!(s.get_event("e0").unwrap().is_none());
}

#[test]
fn nip01_addressable_replaces_by_kind_pubkey_dtag() {
    let s = Store::open_memory().unwrap();
    let mk = |id: &str, ts: u64, d: &str| RelayEvent {
        id: id.into(),
        kind: 30078,
        pubkey: "pk".into(),
        created_at: ts,
        channel_h: String::new(),
        d_tag: d.into(),
        content: String::new(),
        tags_json: "[]".into(),
    };
    assert!(s.insert_event(&mk("a", 1, "d1")).unwrap());
    assert!(s.insert_event(&mk("b", 1, "d2")).unwrap());
    // Replace d1 only; d2 survives (different coordinate).
    assert!(s.insert_event(&mk("c", 2, "d1")).unwrap());
    assert!(s.get_event("a").unwrap().is_none());
    assert!(s.get_event("b").unwrap().is_some());
    assert!(s.get_event("c").unwrap().is_some());
}

#[test]
fn nip01_regular_appends() {
    let s = Store::open_memory().unwrap();
    let mk = |id: &str| RelayEvent {
        id: id.into(),
        kind: 1,
        pubkey: "pk".into(),
        created_at: 1,
        channel_h: "h1".into(),
        d_tag: String::new(),
        content: String::new(),
        tags_json: "[]".into(),
    };
    assert!(s.insert_event(&mk("n1")).unwrap());
    assert!(s.insert_event(&mk("n2")).unwrap());
    assert_eq!(s.chat_for_channel("h1", 0, 10).unwrap().len(), 2);
}

// ── mosaico#743: the tie-break ────────────────────────────────────────────────

/// NIP-01, and NMP's own store, break a `created_at` tie by LOWEST event id.
/// Mosaico kept whichever event arrived first, so the same two events produced
/// different winners in the two stores depending on delivery order.
///
/// The rule this pins is `nmp_store::address_key::candidate_wins`:
///
/// ```text
/// Ordering::Greater => true,
/// Ordering::Less    => false,
/// Ordering::Equal   => candidate.id < current.id,
/// ```
///
/// It is pinned by restatement rather than by calling it, because a consumer
/// CANNOT call it: `candidate_wins` is `pub(crate)` and `nmp_store::EventStore`
/// is re-exported from `nmp` only under the `unstable-mechanism` feature. That
/// unreachability is the reason a second implementation drifted at all, and it
/// is the upstream half of mosaico#743.
#[test]
fn nip01_addressable_tie_is_broken_by_lowest_id_in_either_arrival_order() {
    let mk = |id: &str| RelayEvent {
        id: id.into(),
        kind: 30078,
        pubkey: "pk".into(),
        created_at: 1_700_000_000,
        channel_h: String::new(),
        d_tag: "coordinate".into(),
        content: id.into(),
        tags_json: "[]".into(),
    };

    // Higher id first: the later, lower-id event must displace it.
    let s = Store::open_memory().unwrap();
    assert!(s.insert_event(&mk("bbbb")).unwrap());
    assert!(
        s.insert_event(&mk("aaaa")).unwrap(),
        "the lower id wins even though it arrived second"
    );
    assert!(s.get_event("bbbb").unwrap().is_none());
    assert_eq!(s.get_event("aaaa").unwrap().unwrap().content, "aaaa");

    // Lower id first: the later, higher-id event must lose.
    let s = Store::open_memory().unwrap();
    assert!(s.insert_event(&mk("aaaa")).unwrap());
    assert!(
        !s.insert_event(&mk("bbbb")).unwrap(),
        "the higher id loses even though it arrived second"
    );
    assert!(s.get_event("bbbb").unwrap().is_none());
    assert_eq!(s.get_event("aaaa").unwrap().unwrap().content, "aaaa");
}

#[test]
fn nip01_replaceable_tie_is_broken_by_lowest_id_in_either_arrival_order() {
    let mk = |id: &str| RelayEvent {
        id: id.into(),
        kind: 10002,
        pubkey: "pk".into(),
        created_at: 1_700_000_000,
        channel_h: String::new(),
        d_tag: String::new(),
        content: id.into(),
        tags_json: "[]".into(),
    };

    let s = Store::open_memory().unwrap();
    assert!(s.insert_event(&mk("ffff")).unwrap());
    assert!(s.insert_event(&mk("0000")).unwrap());
    assert_eq!(s.get_event("0000").unwrap().unwrap().content, "0000");
    assert!(s.get_event("ffff").unwrap().is_none());

    let s = Store::open_memory().unwrap();
    assert!(s.insert_event(&mk("0000")).unwrap());
    assert!(!s.insert_event(&mk("ffff")).unwrap());
    assert_eq!(s.get_event("0000").unwrap().unwrap().content, "0000");
    assert!(s.get_event("ffff").unwrap().is_none());
}

/// `created_at` still dominates the id: a strictly newer event with a
/// higher id must win, or the tie-break would have been applied where no tie
/// exists.
#[test]
fn nip01_created_at_dominates_the_tie_break() {
    let mk = |id: &str, created_at: u64| RelayEvent {
        id: id.into(),
        kind: 30078,
        pubkey: "pk".into(),
        created_at,
        channel_h: String::new(),
        d_tag: "coordinate".into(),
        content: id.into(),
        tags_json: "[]".into(),
    };
    let s = Store::open_memory().unwrap();
    assert!(s.insert_event(&mk("aaaa", 100)).unwrap());
    assert!(s.insert_event(&mk("ffff", 200)).unwrap());
    assert_eq!(s.get_event("ffff").unwrap().unwrap().content, "ffff");
    assert!(s.get_event("aaaa").unwrap().is_none());

    // …and an older event with a lower id must still lose.
    assert!(!s.insert_event(&mk("0000", 100)).unwrap());
    assert_eq!(s.get_event("ffff").unwrap().unwrap().content, "ffff");
}

/// Re-delivering an event already cached is a no-op, not a delete-and-
/// reinsert. `id <= ?` rather than `id < ?` is what makes this true.
#[test]
fn nip01_redelivering_the_current_event_is_a_no_op() {
    let event = RelayEvent {
        id: "aaaa".into(),
        kind: 30078,
        pubkey: "pk".into(),
        created_at: 1_700_000_000,
        channel_h: String::new(),
        d_tag: "coordinate".into(),
        content: "body".into(),
        tags_json: "[]".into(),
    };
    let s = Store::open_memory().unwrap();
    assert!(s.insert_event(&event).unwrap());
    assert!(!s.insert_event(&event).unwrap());
    assert_eq!(s.get_event("aaaa").unwrap().unwrap().content, "body");
}
