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
fn hydrated_remote_session_handle_blocks_local_allocation() {
    let store = Store::open_memory().unwrap();
    let wanted_codename = candidates("local-new").next().unwrap();
    let wanted_handle = crate::idref::session_handle("codex", &wanted_codename);
    store
        .upsert_profile_with_agent_slug(
            "remote-pubkey",
            &wanted_handle,
            &wanted_handle,
            "codex",
            "remote-backend",
            false,
            10,
        )
        .unwrap();

    let allocated = allocate(&store, "local-new", 20);
    assert_ne!(allocated.handle, wanted_handle);
}

#[test]
fn hydrated_remote_session_handle_blocks_custom_name_until_retired() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_profile_with_agent_slug(
            "remote-pubkey",
            "quill-codex",
            "quill-codex",
            "codex",
            "remote-backend",
            false,
            10,
        )
        .unwrap();

    let error = store
        .ensure_custom_handle_available("codex", "quill")
        .expect_err("hydrated remote handle must reserve the custom name");
    assert!(error.to_string().contains("already in use"));

    store
        .upsert_profile_with_agent_slug(
            "remote-pubkey",
            "npub1retired",
            "npub1retired",
            "codex",
            "remote-backend",
            false,
            20,
        )
        .unwrap();
    store
        .ensure_custom_handle_available("codex", "quill")
        .expect("retired profile releases its old handle");
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

#[test]
fn custom_name_becomes_the_public_handle_prefix() {
    let store = Store::open_memory().unwrap();
    let allocation = store
        .allocate_custom_handle("pk", "codex", "forensic-researcher", 10)
        .unwrap();

    assert_eq!(allocation.handle, "forensic-researcher-codex");
}

#[test]
fn custom_name_rejects_an_existing_handle() {
    let store = Store::open_memory().unwrap();
    store
        .allocate_custom_handle("first", "codex", "forensic-researcher", 10)
        .unwrap();
    let error = store
        .allocate_custom_handle("second", "codex", "forensic-researcher", 11)
        .unwrap_err();

    assert!(
        error.to_string().contains("forensic-researcher-codex"),
        "unexpected error: {error:#}"
    );
}
