use super::*;
use crate::state::{RegisterSession, StopReason, Store};

#[path = "tests/attachments.rs"]
mod attachments;
#[path = "tests/reaction.rs"]
mod reaction;

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
    use crate::row::{Row, RowDelta};
    use nostr::event::Tag;
    use nostr::event::{EventBuilder, FinalizeEvent, Kind};
    use nostr::key::Keys;

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
            .finalize(&local)
            .unwrap(),
    };
    let views = state.fava().views_handle();

    let first = views.apply_frame("mosaico-h-room", 1, vec![RowDelta::Added(row.clone())]);
    apply_transition(&state, first).await;
    let repeated = views.apply_frame("mosaico-h-room", 2, vec![RowDelta::Added(row)]);
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
    use crate::row::{Row, RowDelta};
    use nostr::event::Tag;
    use nostr::event::{EventBuilder, FinalizeEvent, Kind};
    use nostr::key::Keys;

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
            .finalize(&remote)
            .unwrap(),
    };
    let id = row.event.id;
    let views = state.fava().views_handle();

    let added = views.apply_frame("mosaico-broad-h", 1, vec![RowDelta::Added(row)]);
    apply_transition(&state, added).await;
    while tail.try_recv().is_ok() {}

    let removed = views.apply_frame("mosaico-broad-h", 1, vec![RowDelta::Removed(id)]);
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
    use crate::row::{Row, RowDelta};
    use nostr::event::Tag;
    use nostr::event::{EventBuilder, FinalizeEvent, Kind};
    use nostr::key::Keys;

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
            .finalize(&remote)
            .unwrap(),
    };
    let id = row.event.id;
    let views = state.fava().views_handle();

    // Under the single broad observation, one entry fans out to all channels
    // named in the status event's #h tags simultaneously.
    let entered = views.apply_frame("mosaico-broad-h", 1, vec![RowDelta::Added(row.clone())]);
    assert_eq!(
        entered.added.len(),
        1,
        "first entry adds to the canonical union"
    );
    assert_eq!(entered.entered.len(), 1, "single broad observation edge");
    apply_transition(&state, entered).await;
    let ab_events = [
        tail.try_recv().unwrap(),
        tail.try_recv().unwrap(),
        tail.try_recv().unwrap(),
        tail.try_recv().unwrap(),
    ];
    assert!(
        ab_events
            .iter()
            .any(|e| matches!(e, TailEvent::Join { channel, .. } if channel == "room-a")),
        "broad entry emits Join for room-a"
    );
    assert!(
        ab_events
            .iter()
            .any(|e| matches!(e, TailEvent::Status { channel, .. } if channel == "room-a")),
        "broad entry emits Status for room-a"
    );
    assert!(
        ab_events
            .iter()
            .any(|e| matches!(e, TailEvent::Join { channel, .. } if channel == "room-b")),
        "broad entry emits Join for room-b"
    );
    assert!(
        ab_events
            .iter()
            .any(|e| matches!(e, TailEvent::Status { channel, .. } if channel == "room-b")),
        "broad entry emits Status for room-b"
    );
    assert!(tail.try_recv().is_err(), "exactly four status events");

    // One broad departure fans out Leaves to all channels the status named.
    let departed = views.apply_frame("mosaico-broad-h", 1, vec![RowDelta::Removed(id)]);
    apply_transition(&state, departed).await;
    let leave_events = [tail.try_recv().unwrap(), tail.try_recv().unwrap()];
    assert!(
        leave_events
            .iter()
            .all(|e| matches!(e, TailEvent::Leave { .. })),
        "both channel departures emit Leave"
    );
    assert!(
        leave_events
            .iter()
            .any(|e| matches!(e, TailEvent::Leave { channel, .. } if channel == "room-a")),
        "broad departure emits Leave for room-a"
    );
    assert!(
        leave_events
            .iter()
            .any(|e| matches!(e, TailEvent::Leave { channel, .. } if channel == "room-b")),
        "broad departure emits Leave for room-b"
    );
    assert!(tail.try_recv().is_err());
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
