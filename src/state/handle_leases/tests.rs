use super::*;

fn allocate(store: &Store, pubkey: &str, now: u64) -> HandleAllocation {
    store.allocate_handle(pubkey, "codex", now).unwrap()
}

#[test]
fn first_allocation_uses_one_word_tier_and_resume_keeps_it() {
    let store = Store::open_memory().unwrap();
    let first = allocate(&store, "pk-one", 10);
    assert_eq!(first.handle.split('-').count(), 2);
    store
        .conn
        .execute("UPDATE handle_leases SET live=0 WHERE pubkey='pk-one'", [])
        .unwrap();
    assert_eq!(
        allocate(&store, "pk-one", 10 + HANDLE_LEASE_GRACE_SECS + 1).handle,
        first.handle,
        "resume before actual reclamation keeps the expired lease"
    );
}

#[test]
fn exhausts_every_one_word_candidate_before_tier_two() {
    let store = Store::open_memory().unwrap();
    for i in 0..64 {
        let allocated = allocate(&store, &format!("occupied-{i}"), 10);
        assert_eq!(allocated.handle.split('-').count(), 2);
    }
    let tier_two = allocate(&store, "next", 10);
    assert_eq!(tier_two.handle.split('-').count(), 3);
}

#[test]
fn seven_day_boundary_is_lazy_and_atomic() {
    let store = Store::open_memory().unwrap();
    let old = allocate(&store, "old", 100);
    store
        .upsert_identity(&Identity {
            pubkey: "old".into(),
            agent_slug: "codex".into(),
            codename: old.codename.clone(),
            session_id: "old-session".into(),
            channel_h: "root".into(),
            native_id: "native-old".into(),
            alive: false,
            created_at: 100,
        })
        .unwrap();
    store
        .conn
        .execute("UPDATE handle_leases SET live=0 WHERE pubkey='old'", [])
        .unwrap();
    for i in 0..63 {
        let occupied = allocate(&store, &format!("occupied-{i}"), 100);
        assert_ne!(occupied.handle, old.handle);
    }
    let before = allocate(&store, "before", 100 + HANDLE_LEASE_GRACE_SECS - 1);
    assert_ne!(before.handle, old.handle);
    assert_eq!(before.handle.split('-').count(), 3);

    let at_boundary = allocate(&store, "boundary", 100 + HANDLE_LEASE_GRACE_SECS);
    assert_eq!(at_boundary.handle, old.handle);
    assert_eq!(at_boundary.reclaimed_pubkey.as_deref(), Some("old"));
    assert_eq!(
        store.pubkey_for_handle(&old.handle).unwrap().as_deref(),
        Some("boundary")
    );
    assert!(store.handle_for_pubkey("old").unwrap().is_none());
    assert_eq!(store.get_identity("old").unwrap().unwrap().codename, "");

    let resumed_old = allocate(&store, "old", 100 + HANDLE_LEASE_GRACE_SECS + 1);
    assert_ne!(resumed_old.handle, old.handle);
    assert_eq!(
        store
            .pubkey_for_handle(&resumed_old.handle)
            .unwrap()
            .as_deref(),
        Some("old")
    );
}

#[test]
fn existing_long_identity_is_backfilled_as_a_tier_three_lease() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_identity(&Identity {
            pubkey: "legacy-pubkey".into(),
            agent_slug: "codex".into(),
            codename: "amber-arrow-007".into(),
            session_id: "legacy-session".into(),
            channel_h: "root".into(),
            native_id: "native-legacy".into(),
            alive: false,
            created_at: 10,
        })
        .unwrap();
    store.backfill_handle_leases().unwrap();
    assert_eq!(
        store.handle_for_pubkey("legacy-pubkey").unwrap().as_deref(),
        Some("amber-arrow-007-codex")
    );
}

#[test]
fn candidate_space_finishes_tier_two_before_tier_three() {
    let generated = candidates("seed").take(64 + 64_000 + 1).collect::<Vec<_>>();
    assert!(generated[..64]
        .iter()
        .all(|name| name.split('-').count() == 1));
    assert!(generated[64..64 + 64_000]
        .iter()
        .all(|name| name.split('-').count() == 2));
    assert_eq!(generated.last().unwrap().split('-').count(), 3);
}

#[test]
fn concurrent_connections_never_allocate_the_same_handle() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    Store::open(&path).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let threads = (0..8)
        .map(|i| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let store = Store::open(&path).unwrap();
                barrier.wait();
                allocate(&store, &format!("concurrent-{i}"), 10).handle
            })
        })
        .collect::<Vec<_>>();
    let mut handles = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();
    handles.sort();
    handles.dedup();
    assert_eq!(handles.len(), 8);
}
