use super::*;
use crate::reconcile::HookContextState;

#[test]
fn membership_delta_uses_the_same_members_block_with_plain_departure_prose() {
    let store = seed_store();
    let rec = session(&store);
    store
        .upsert_status(&crate::state::Status {
            pubkey: OTHER_PK.into(),
            channel_h: "root".into(),
            slug: "amber-reviewer".into(),
            title: "Reviewing".into(),
            activity: String::new(),
            workspace: "root".into(),
            branch: String::new(),
            state: crate::session_state::SessionState::Idle,
            state_since: 90,
            last_seen: 90,
            updated_at: 90,
            expiration: 500,
        })
        .unwrap();
    let before = capture_inputs(&store, &input(Some(&rec), "root", 0, 100, true)).unwrap();
    let mut hook = HookContextState::default();
    hook.render_context(&rec.pubkey, "turn_start", 0, 100, before);

    store
        .replace_channel_members("root", &[SELF_PK.into()], 101)
        .unwrap();
    let after = capture_inputs(&store, &input(Some(&rec), "root", 100, 200, false)).unwrap();
    let delta = hook
        .render_context(&rec.pubkey, "turn_start", 100, 200, after)
        .text
        .expect("a confirmed roster departure should produce a delta");

    assert!(delta.contains("<members>"), "{delta}");
    assert!(delta.contains("@amber-reviewer left."), "{delta}");
    assert!(!delta.contains("@reviewer left."), "{delta}");
    assert!(!delta.contains("op="), "{delta}");
    assert!(!delta.contains("<recent-presence"), "{delta}");
}

#[test]
fn a_late_status_behind_the_cursor_still_emits_a_member_delta() {
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
            title: "Reviewing the late event".into(),
            activity: "Checking deltas".into(),
            workspace: "root".into(),
            branch: "feat/context".into(),
            state: crate::session_state::SessionState::Working,
            state_since: 150,
            last_seen: 150,
            updated_at: 150,
            expiration: 500,
        })
        .unwrap();
    let after = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    assert!(
        assemble::assemble_view(&after, 200, 300).is_empty(),
        "cursor-only assembly should reproduce the late-arrival race"
    );
    let delta = hook
        .render_context(&rec.pubkey, "turn_start", 200, 300, after)
        .text
        .expect("the frozen-input comparison must recover the late status");

    assert!(delta.contains("<members>"), "{delta}");
    assert!(
        delta.contains(
            "<agent name=\"@reviewer\" branch=\"feat/context\" state=\"working\" \
             status=\"Checking deltas\" since=\"2 min ago\" />"
        ),
        "{delta}"
    );
    let quiet = capture_inputs(&store, &input(Some(&rec), "root", 300, 400, false)).unwrap();
    assert!(
        hook.render_context(&rec.pubkey, "turn_start", 300, 400, quiet)
            .text
            .is_none(),
        "the recovered semantic row must emit exactly once"
    );
}

#[test]
fn a_late_roster_addition_behind_the_cursor_still_emits_a_member_delta() {
    let store = seed_store();
    store
        .replace_channel_members("root", &[SELF_PK.into()], 100)
        .unwrap();
    let rec = session(&store);
    let mut hook = HookContextState::default();
    let before = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    hook.render_context(&rec.pubkey, "turn_start", 200, 300, before);

    store
        .replace_channel_members("root", &[SELF_PK.into(), OTHER_PK.into()], 150)
        .unwrap();
    let after = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    let delta = hook
        .render_context(&rec.pubkey, "turn_start", 200, 300, after)
        .text
        .expect("the frozen-input comparison must recover the late roster row");

    assert!(delta.contains("<members>"), "{delta}");
    assert!(delta.contains("<agent name=\"@reviewer\" />"), "{delta}");
}

#[test]
fn an_unjoined_descendant_never_emits_typed_member_detail() {
    let store = seed_store();
    let rec = session(&store);
    store
        .revoke_route_and_mark_absent(&rec.pubkey, TASK_H, 100)
        .unwrap();
    let mut hook = HookContextState::default();
    let before = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    hook.render_context(&rec.pubkey, "turn_start", 200, 300, before);

    store
        .upsert_status(&crate::state::Status {
            pubkey: OTHER_PK.into(),
            channel_h: TASK_H.into(),
            slug: "reviewer".into(),
            title: "Private child work".into(),
            activity: "Should not leak".into(),
            workspace: "root".into(),
            branch: String::new(),
            state: crate::session_state::SessionState::Working,
            state_since: 150,
            last_seen: 150,
            updated_at: 150,
            expiration: 500,
        })
        .unwrap();
    let after = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    assert!(
        hook.render_context(&rec.pubkey, "turn_start", 200, 300, after)
            .text
            .is_none(),
        "an unjoined child status must not create a typed member delta"
    );

    let full = render_fabric_context(&store, input(Some(&rec), "root", 0, 300, false))
        .expect("the descendant metadata remains visible");
    assert!(
        full.contains("<channel name=\"/root/task\" about=\"Task room\" agents=\"2\" />"),
        "{full}"
    );
    assert!(!full.contains("Private child work"), "{full}");
    assert!(!full.contains("Should not leak"), "{full}");
}

#[test]
fn a_status_without_confirmed_roster_membership_cannot_manufacture_a_member() {
    let store = seed_store();
    let rec = session(&store);
    let mut hook = HookContextState::default();
    let before = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    hook.render_context(&rec.pubkey, "turn_start", 200, 300, before);

    store
        .upsert_status(&crate::state::Status {
            pubkey: "not-a-member".into(),
            channel_h: "root".into(),
            slug: "impostor".into(),
            title: "Claimed work".into(),
            activity: "Unconfirmed".into(),
            workspace: "root".into(),
            branch: String::new(),
            state: crate::session_state::SessionState::Working,
            state_since: 150,
            last_seen: 150,
            updated_at: 150,
            expiration: 500,
        })
        .unwrap();
    let after = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    assert!(
        hook.render_context(&rec.pubkey, "turn_start", 200, 300, after)
            .text
            .is_none(),
        "status alone is not channel membership"
    );
}
