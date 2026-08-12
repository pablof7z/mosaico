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

#[tokio::test]
async fn observed_local_authored_chat_emits_tail_exactly_once() {
    use nmp::{Row, RowDelta};
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use std::collections::BTreeSet;

    let state = DaemonState::new_for_test().await;
    let local = Keys::generate();
    state.with_store(|store| {
        register(
            store,
            &local.public_key().to_hex(),
            "local-codex",
            "room",
            "locator",
        );
    });
    let mut tail = state.tail_subscribe();
    let row = Row {
        event: EventBuilder::new(Kind::from(9_u16), "observed once")
            .tags([Tag::parse(["h", "room"]).unwrap()])
            .sign_with_keys(&local)
            .unwrap(),
        sources: BTreeSet::new(),
    };
    let views = state.nmp().views_handle();

    let first = views.apply_frame(
        "mosaico-h-room",
        1,
        vec![RowDelta::Added(row.clone())],
        vec![],
    );
    apply_transition(&state, first).await;
    let repeated = views.apply_frame("mosaico-h-room", 2, vec![RowDelta::Added(row)], vec![]);
    apply_transition(&state, repeated).await;

    let event = tail
        .try_recv()
        .expect("observed local chat emits a tail row");
    assert!(matches!(
        event,
        TailEvent::Msg { channel, body, .. }
            if channel == "room" && body == "observed once"
    ));
    assert!(
        tail.try_recv().is_err(),
        "repeated observation must not emit a second tail row"
    );
}

#[tokio::test]
async fn departed_status_row_emits_leave_without_waiting_for_a_poll() {
    use nmp::{Row, RowDelta};
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use std::collections::BTreeSet;

    let state = DaemonState::new_for_test().await;
    let mut tail = state.tail_subscribe();
    let remote = Keys::generate();
    let row = Row {
        event: EventBuilder::new(Kind::from(30315_u16), "working")
            .tags([
                Tag::parse(["d", "status"]).unwrap(),
                Tag::parse(["h", "room"]).unwrap(),
                Tag::parse(["title", "Focused task"]).unwrap(),
                Tag::parse(["state", "working"]).unwrap(),
                Tag::parse(["state-since", "1"]).unwrap(),
                Tag::parse(["host", "remote-host"]).unwrap(),
                Tag::parse(["workspace", "room"]).unwrap(),
                Tag::parse(["slug", "remote-codex"]).unwrap(),
            ])
            .sign_with_keys(&remote)
            .unwrap(),
        sources: BTreeSet::new(),
    };
    let id = row.event.id;
    let views = state.nmp().views_handle();

    let added = views.apply_frame("mosaico-h-room", 1, vec![RowDelta::Added(row)], vec![]);
    apply_transition(&state, added).await;
    while tail.try_recv().is_ok() {}

    let removed = views.apply_frame("mosaico-h-room", 1, vec![RowDelta::Removed(id)], vec![]);
    apply_transition(&state, removed).await;

    let leave = tail.try_recv().expect("status departure emits immediately");
    assert!(matches!(
        leave,
        TailEvent::Leave {
            channel,
            agent,
            host,
            session,
            ..
        } if channel == "room"
            && agent == "remote-codex"
            && host == "remote-host"
            && session == remote.public_key().to_hex()
    ));
}

#[tokio::test]
async fn status_tail_events_follow_exact_observation_edges() {
    use nmp::{Row, RowDelta};
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use std::collections::BTreeSet;

    let state = DaemonState::new_for_test().await;
    let mut tail = state.tail_subscribe();
    let remote = Keys::generate();
    let row = Row {
        event: EventBuilder::new(Kind::from(30315_u16), "working")
            .tags([
                Tag::parse(["d", "status"]).unwrap(),
                Tag::parse(["h", "room-a"]).unwrap(),
                Tag::parse(["h", "room-b"]).unwrap(),
                Tag::parse(["title", "Focused task"]).unwrap(),
                Tag::parse(["state", "working"]).unwrap(),
                Tag::parse(["state-since", "1"]).unwrap(),
                Tag::parse(["host", "remote-host"]).unwrap(),
                Tag::parse(["workspace", "room-a"]).unwrap(),
                Tag::parse(["slug", "remote-codex"]).unwrap(),
            ])
            .sign_with_keys(&remote)
            .unwrap(),
        sources: BTreeSet::new(),
    };
    let id = row.event.id;
    let views = state.nmp().views_handle();

    let entered_a = views.apply_frame(
        "mosaico-h-room-a",
        1,
        vec![RowDelta::Added(row.clone())],
        vec![],
    );
    apply_transition(&state, entered_a).await;
    let a_events = [tail.try_recv().unwrap(), tail.try_recv().unwrap()];
    assert!(a_events.iter().all(|event| matches!(
        event,
        TailEvent::Join { channel, .. } | TailEvent::Status { channel, .. }
            if channel == "room-a"
    )));
    assert!(tail.try_recv().is_err(), "room-b is not observed yet");

    let entered_b = views.apply_frame("mosaico-h-room-b", 1, vec![RowDelta::Added(row)], vec![]);
    assert!(
        entered_b.added.is_empty(),
        "the canonical Row already exists"
    );
    assert_eq!(entered_b.entered.len(), 1, "the observation edge is new");
    apply_transition(&state, entered_b).await;
    let b_events = [tail.try_recv().unwrap(), tail.try_recv().unwrap()];
    assert!(b_events.iter().all(|event| matches!(
        event,
        TailEvent::Join { channel, .. } | TailEvent::Status { channel, .. }
            if channel == "room-b"
    )));
    assert!(tail.try_recv().is_err());

    let departed_b = views.apply_frame("mosaico-h-room-b", 1, vec![RowDelta::Removed(id)], vec![]);
    apply_transition(&state, departed_b).await;
    assert!(matches!(
        tail.try_recv().unwrap(),
        TailEvent::Leave { channel, .. } if channel == "room-b"
    ));
    assert!(tail.try_recv().is_err(), "room-a remains observed");
    state.with_store(|store| {
        assert!(store
            .get_status(&remote.public_key().to_hex(), "room-a")
            .unwrap()
            .is_some());
        assert!(store
            .get_status(&remote.public_key().to_hex(), "room-b")
            .unwrap()
            .is_none());
    });
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
