use super::*;
use crate::reconcile::HookContextState;

#[test]
fn a_heartbeat_only_refresh_does_not_emit_a_member_delta() {
    let store = seed_store();
    let rec = session(&store);
    let mut peer = crate::state::Status {
        pubkey: OTHER_PK.into(),
        channel_h: "root".into(),
        slug: "reviewer".into(),
        title: "Reviewing".into(),
        activity: String::new(),
        workspace: "root".into(),
        branch: String::new(),
        state: crate::session_state::SessionState::Idle,
        state_since: 100,
        last_seen: 150,
        updated_at: 150,
        expiration: 500,
    };
    store.upsert_status(&peer).unwrap();
    let mut hook = HookContextState::default();
    let before = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    hook.render_context(&rec.pubkey, "turn_start", 200, 300, before);

    peer.last_seen = 250;
    peer.updated_at = 250;
    peer.expiration = 600;
    store.upsert_status(&peer).unwrap();
    let after = capture_inputs(&store, &input(Some(&rec), "root", 300, 400, false)).unwrap();
    let outcome = hook.render_context(&rec.pubkey, "turn_start", 300, 400, after);

    assert!(
        outcome.text.is_none(),
        "lease renewal alone should stay quiet: {:?}",
        outcome.text
    );
}

#[test]
fn a_replacement_heartbeat_only_refresh_does_not_emit_a_member_delta() {
    let store = seed_store();
    let rec = session(&store);
    let mut peer = crate::state::Status {
        pubkey: OTHER_PK.into(),
        channel_h: "root".into(),
        slug: "reviewer".into(),
        title: "Reviewing".into(),
        activity: String::new(),
        workspace: "root".into(),
        branch: String::new(),
        state: crate::session_state::SessionState::Idle,
        state_since: 100,
        last_seen: 150,
        updated_at: 150,
        expiration: 500,
    };
    store
        .replace_status_channels(OTHER_PK, &[peer.clone()], 150)
        .unwrap();
    let mut hook = HookContextState::default();
    let before = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    hook.render_context(&rec.pubkey, "turn_start", 200, 300, before);

    peer.last_seen = 250;
    peer.updated_at = 250;
    peer.expiration = 600;
    store
        .replace_status_channels(OTHER_PK, &[peer], 250)
        .unwrap();
    let after = capture_inputs(&store, &input(Some(&rec), "root", 300, 400, false)).unwrap();
    let outcome = hook.render_context(&rec.pubkey, "turn_start", 300, 400, after);

    assert!(
        outcome.text.is_none(),
        "replacement lease renewal alone should stay quiet: {:?}",
        outcome.text
    );
}

#[test]
fn a_future_dated_status_is_not_recovered_before_its_time() {
    let store = seed_store();
    let rec = session(&store);
    let mut hook = HookContextState::default();
    let before = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    hook.render_context(&rec.pubkey, "turn_start", 200, 300, before);

    store
        .upsert_status(&crate::state::Status {
            pubkey: OTHER_PK.into(),
            channel_h: "root".into(),
            slug: "reviewer".into(),
            title: "Future work".into(),
            activity: String::new(),
            workspace: "root".into(),
            branch: String::new(),
            state: crate::session_state::SessionState::Idle,
            state_since: 350,
            last_seen: 350,
            updated_at: 350,
            expiration: 500,
        })
        .unwrap();
    let future = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    assert!(
        hook.render_context(&rec.pubkey, "turn_start", 200, 300, future)
            .text
            .is_none(),
        "future status facts must fail closed"
    );
}

#[test]
fn a_fresh_hook_cache_rebaselines_members_without_replaying_chatter() {
    let store = seed_store();
    let rec = session(&store);
    store
        .upsert_status(&crate::state::Status {
            pubkey: OTHER_PK.into(),
            channel_h: "root".into(),
            slug: "reviewer".into(),
            title: "Reviewing before restart".into(),
            activity: String::new(),
            workspace: "root".into(),
            branch: "feat/restart".into(),
            state: crate::session_state::SessionState::Idle,
            state_since: 100,
            last_seen: 150,
            updated_at: 150,
            expiration: 500,
        })
        .unwrap();
    record(&store, "old-chat", "root", "accepted", 150);
    let captured = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    let mut hook = HookContextState::default();
    let text = hook
        .render_context(&rec.pubkey, "turn_start", 200, 300, captured)
        .text
        .expect("a new presentation cache must restore the joined roster");

    assert!(
        text.contains(
            "<agent name=\"@reviewer\" branch=\"feat/restart\" state=\"idle\" \
             status=\"Reviewing before restart\" since=\"3 min ago\" />"
        ),
        "{text}"
    );
    assert!(!text.contains("<chatter>"), "{text}");
    assert!(!text.contains("hello"), "{text}");
}
