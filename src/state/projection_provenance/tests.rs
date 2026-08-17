use super::*;

#[test]
fn one_observation_removal_does_not_retract_another_observations_row() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_profile("peer", "Peer", "peer", "remote", false, 1)
        .unwrap();
    store
        .set_projection_source(ProjectionKind::Profile, "peer", "event-a")
        .unwrap();
    for observation in ["profiles", "mentions"] {
        store
            .claim_projection_event(observation, 1, "event-a", "[]")
            .unwrap();
    }

    assert!(!store
        .release_projection_event("profiles", "event-a")
        .unwrap());
    assert!(!store.retract_projection_source("event-a").unwrap());
    assert!(store.get_profile("peer").unwrap().is_some());

    assert!(store
        .release_projection_event("mentions", "event-a")
        .unwrap());
    assert!(store.retract_projection_source("event-a").unwrap());
    assert!(store.get_profile("peer").unwrap().is_none());
}

#[test]
fn settled_generation_retracts_only_rows_not_seen_again() {
    let store = Store::open_memory().unwrap();
    for (pubkey, event_id) in [("kept", "event-a"), ("gone", "event-b")] {
        store
            .upsert_profile(pubkey, pubkey, pubkey, "remote", false, 1)
            .unwrap();
        store
            .set_projection_source(ProjectionKind::Profile, pubkey, event_id)
            .unwrap();
        store
            .claim_projection_event("profiles", 1, event_id, "[]")
            .unwrap();
    }
    store
        .claim_projection_event("profiles", 2, "event-a", "[]")
        .unwrap();

    let orphaned = store.settle_projection_frame("profiles", 2).unwrap();
    assert_eq!(orphaned, ["event-b"]);
    for event_id in orphaned {
        store.retract_projection_source(&event_id).unwrap();
    }
    assert!(store.get_profile("kept").unwrap().is_some());
    assert!(store.get_profile("gone").unwrap().is_none());
}

#[test]
fn source_retraction_cannot_delete_a_newer_projection_winner() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_profile("peer", "New", "new", "remote", false, 2)
        .unwrap();
    store
        .set_projection_source(ProjectionKind::Profile, "peer", "new-event")
        .unwrap();
    store
        .claim_projection_event("profiles", 2, "new-event", "[]")
        .unwrap();

    assert!(!store.retract_projection_source("old-event").unwrap());
    assert_eq!(store.get_profile("peer").unwrap().unwrap().name, "New");
}
