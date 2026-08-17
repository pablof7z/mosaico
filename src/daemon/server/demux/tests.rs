use super::*;
use crate::state::{RegisterSession, StopReason, Store};

#[path = "tests/attachments.rs"]
mod attachments;

// ── helpers ───────────────────────────────────────────────────────────────────

fn register(store: &Store, pubkey: &str, slug: &str, channel: &str, _locator: &str) -> String {
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: pubkey.into(),
            observed_harness: "claude-code".into(),
            agent_slug: slug.into(),
            launch_channel_h: channel.into(),
            work_root: channel.into(),
            child_pid: Some(42),
            now: 1000,
        })
        .unwrap();
    store.bind_session_signer(pubkey, "test-salt").unwrap();
    pubkey.to_string()
}

// ── mosaico#744: applying an NMP frame as a unit ─────────────────────────────

fn cached(store: &Store, id: &str) -> bool {
    store.has_event(id).unwrap()
}

fn chat_row(id: &str) -> crate::state::RelayEvent {
    crate::state::RelayEvent {
        id: id.into(),
        kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
        pubkey: "pk".into(),
        created_at: 10,
        channel_h: "room".into(),
        d_tag: String::new(),
        content: "hello".into(),
        tags_json: "[]".into(),
    }
}

#[test]
fn a_retraction_removes_exactly_the_named_row() {
    let store = Store::open_memory().unwrap();
    let doomed = event_id("aa").to_hex();
    let bystander = event_id("bb").to_hex();
    store.insert_event(&chat_row(&doomed)).unwrap();
    store.insert_event(&chat_row(&bystander)).unwrap();
    store
        .set_projection_source(crate::state::ProjectionKind::Event, &doomed, &doomed)
        .unwrap();
    store
        .claim_projection_event("contents", 1, &doomed, "[]")
        .unwrap();

    assert!(store.release_projection_event("contents", &doomed).unwrap());
    store.retract_projection_source(&doomed).unwrap();

    assert!(
        !cached(&store, &doomed),
        "a retracted event must leave the cache"
    );
    assert!(cached(&store, &bystander), "an untouched row must survive");
}

/// The order is load-bearing, not stylistic. NMP composes a row that was
/// present at the baseline, removed, and re-added into a `Replaced`
/// transition, delivered as `Removed(id)` followed by `Added(row)` for the
/// SAME id. Applying additions first and removals second deletes that row
/// outright — the batch's own addition, undone by its own removal.
#[test]
fn removals_are_applied_before_additions_so_a_replaced_row_survives() {
    let store = Store::open_memory().unwrap();
    let id = event_id("cc");
    let id = id.to_hex();

    // Removals first, then the addition: the row is present afterwards.
    store
        .claim_projection_event("contents", 1, &id, "[]")
        .unwrap();
    store.release_projection_event("contents", &id).unwrap();
    store.retract_projection_source(&id).unwrap();
    store.insert_event(&chat_row(&id)).unwrap();
    store
        .set_projection_source(crate::state::ProjectionKind::Event, &id, &id)
        .unwrap();
    store
        .claim_projection_event("contents", 1, &id, "[]")
        .unwrap();
    assert!(
        cached(&store, &id),
        "a Removed+Added pair for one id must leave the row present"
    );

    // The order this replaces: the addition, then the removal that was meant
    // to precede it. The row is gone.
    store.insert_event(&chat_row(&id)).unwrap();
    store.release_projection_event("contents", &id).unwrap();
    store.retract_projection_source(&id).unwrap();
    assert!(
        !cached(&store, &id),
        "NOTHING TO OBSERVE — additions-first must actually lose the row, \
         otherwise this test proves nothing about the ordering"
    );
}

fn event_id(prefix: &str) -> nostr::EventId {
    nostr::EventId::from_hex(&format!("{prefix}{}", "0".repeat(64 - prefix.len()))).unwrap()
}

// ── has_alive gate ────────────────────────────────────────────────────────────

#[test]
fn has_alive_gate_skips_when_agent_has_live_session() {
    let store = Store::open_memory().unwrap();
    let sid = register(&store, "pk-ord-1", "codex", "proj", "ext-1");
    // reserve_session creates a running runtime generation.
    assert!(!sid.is_empty());

    assert!(offline_mention::liveness::has_alive_session_for(
        &store, "pk-ord-1"
    ));
}

#[test]
fn has_alive_gate_does_not_skip_when_session_is_dead() {
    let store = Store::open_memory().unwrap();
    let sid = register(&store, "pk-ord-1", "codex", "proj", "ext-1");
    store
        .mark_runtime_stopped(&sid, StopReason::Crash, 1_001)
        .unwrap();

    assert!(!offline_mention::liveness::has_alive_session_for(
        &store, "pk-ord-1"
    ));
}

#[test]
fn has_alive_gate_skips_when_agent_has_live_session_in_a_different_channel() {
    let store = Store::open_memory().unwrap();
    let _sid = register(&store, "pk-ord-1", "codex", "other-proj", "ext-1");

    assert!(offline_mention::liveness::has_alive_session_for(
        &store, "pk-ord-1"
    ));
}

#[test]
fn has_alive_gate_matches_derived_ordinal_pubkey_not_base() {
    let store = Store::open_memory().unwrap();
    // Session registered with the ordinal pubkey, not the base.
    let _sid = register(&store, "pk-ord-2", "codex", "proj", "ext-2");

    assert!(offline_mention::liveness::has_alive_session_for(
        &store, "pk-ord-2"
    ));
    assert!(!offline_mention::liveness::has_alive_session_for(
        &store, "base-pk"
    ));
}

#[test]
fn has_alive_gate_matches_joined_subchannel_not_just_home_channel() {
    let store = Store::open_memory().unwrap();
    let sid = register(&store, "pk-ord-1", "codex", "proj", "ext-1");
    // Join a sub-channel
    store.grant_session_route(&sid, "sub-chan", 10).unwrap();

    assert!(offline_mention::liveness::has_alive_session_for(
        &store, "pk-ord-1"
    ));
}

// ── eye-reaction routing gate ─────────────────────────────────────────────────

/// Replicates the `hosted.contains(mentioned_pk)` gate in handle_incoming that
/// decides whether to publish the eye reaction.
fn should_publish_eye_reaction(hosted: &[String], mentioned_pk: &str) -> bool {
    hosted.contains(&mentioned_pk.to_string())
}

#[test]
fn eye_reaction_fires_for_hosted_agent_pubkey() {
    let hosted = vec!["pk-ord-1".to_string(), "pk-ord-2".to_string()];
    assert!(should_publish_eye_reaction(&hosted, "pk-ord-1"));
}

#[test]
fn eye_reaction_fires_for_identity_derived_pubkey() {
    // The hosted set includes persisted local session pubkeys.
    let hosted = vec!["base-pk".to_string(), "pk-ord-1".to_string()];
    assert!(should_publish_eye_reaction(&hosted, "pk-ord-1"));
}

#[test]
fn eye_reaction_does_not_fire_for_foreign_peer() {
    let hosted = vec!["pk-ord-1".to_string()];
    assert!(!should_publish_eye_reaction(&hosted, "foreign-pk"));
}

#[test]
fn eye_reaction_does_not_fire_for_empty_mentioned_pk() {
    let hosted = vec!["pk-ord-1".to_string()];
    assert!(!should_publish_eye_reaction(&hosted, ""));
}
