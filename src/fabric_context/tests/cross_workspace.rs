use super::*;
use crate::reconcile::hook_context::HookContextState;

fn add_workspace(store: &Store) {
    store
        .upsert_channel("remote", "general", "Remote room", "", 1)
        .unwrap();
    store
        .upsert_channel("review-h", "review", "Review room", "remote", 1)
        .unwrap();
    for channel in ["remote", "review-h"] {
        store.replace_channel_members(channel, &[], 1).unwrap();
        store.replace_channel_admins(channel, &[], 1).unwrap();
    }
}

fn put_status(
    store: &Store,
    pubkey: &str,
    channel: &str,
    activity: &str,
    updated_at: u64,
    expiration: u64,
) {
    store
        .upsert_status(&Status {
            pubkey: pubkey.into(),
            channel_h: channel.into(),
            slug: "reviewer".into(),
            title: "Reviewing".into(),
            activity: activity.into(),
            workspace: if channel == "root" { "root" } else { "remote" }.into(),
            branch: String::new(),
            state: crate::session_state::SessionState::Working,
            state_since: updated_at,
            last_seen: updated_at,
            updated_at,
            expiration,
        })
        .unwrap();
}

#[test]
fn outside_workspace_is_a_compact_root_and_does_not_leak_presence_or_chatter() {
    let store = seed_store();
    let rec = session(&store);
    add_workspace(&store);
    put_status(&store, OTHER_PK, "remote", "coordinating release", 250, 500);
    put_status(&store, OTHER_PK, "review-h", "reviewing patch", 260, 500);
    chat(
        &store,
        "remote-chat",
        "review-h",
        270,
        "private unjoined chatter",
        "[]",
    );

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false));
    assert!(
        text.is_none(),
        "outside presence alone must not expand the delta: {text:?}"
    );
    let captured = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    assert!(assemble::assemble_view(&captured, 200, 300).is_empty());
    let mut state = HookContextState::default();
    let outcome = state.render_context("sess", "turn_start", 200, 300, captured);
    let baseline = outcome
        .text
        .expect("a fresh hook cache re-baselines the joined root roster");
    assert!(baseline.contains("<channel name=\"/root\""), "{baseline}");
    assert!(
        !baseline.contains("<channel name=\"/remote\""),
        "{baseline}"
    );
    assert!(!baseline.contains("coordinating release"), "{baseline}");
    assert!(!baseline.contains("reviewing patch"), "{baseline}");
    assert!(!baseline.contains("private unjoined chatter"), "{baseline}");

    let full = render_fabric_context(&store, input(Some(&rec), "root", 0, 300, false))
        .expect("current workspace full snapshot");
    assert!(
        full.contains("<channel name=\"/remote\" about=\"Remote room\" agents=\"0\" />"),
        "{full}"
    );
    assert!(!full.contains("/remote/review"), "{full}");
    assert!(!full.contains("coordinating release"), "{full}");
    assert!(!full.contains("reviewing patch"), "{full}");
    assert!(!full.contains("private unjoined chatter"), "{full}");
}

#[test]
fn outside_workspace_departures_do_not_expand_the_compact_root() {
    let store = seed_store();
    let rec = session(&store);
    add_workspace(&store);
    store
        .replace_channel_members("remote", &[OTHER_PK.into()], 150)
        .unwrap();
    let mut state = HookContextState::default();
    let before = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    state.render_context(&rec.pubkey, "turn_start", 200, 300, before);

    store.replace_channel_members("remote", &[], 175).unwrap();
    let after = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    let outcome = state.render_context(&rec.pubkey, "turn_start", 200, 300, after);

    assert!(
        outcome.text.is_none(),
        "an outside departure must not expand /remote: {:?}",
        outcome.text
    );
}

#[test]
fn unscoped_session_sees_known_roots_without_expanding_them() {
    let store = seed_store();
    let rec = session_record(&store, "unscoped", "");
    add_workspace(&store);
    put_status(&store, OTHER_PK, "remote", "coordinating release", 250, 500);

    let text = render_fabric_context(&store, input(Some(&rec), "", 0, 300, true))
        .expect("full briefing should orient an unscoped session");
    assert!(text.contains("<channel name=\"/remote\""), "{text}");
    assert!(!text.contains("/remote/review"), "{text}");
    assert!(!text.contains("coordinating release"), "{text}");
    assert!(!text.contains("<workspace"), "{text}");
}

#[test]
fn current_workspace_delta_omits_outside_workspace_statuses() {
    let store = seed_store();
    let rec = session(&store);
    add_workspace(&store);
    put_status(&store, OTHER_PK, "remote", "old work", 150, 500);
    put_status(&store, OTHER_PK, "remote", "expired work", 250, 299);
    put_status(&store, SELF_PK, "remote", "self work", 250, 500);
    put_status(&store, OTHER_PK, "root", "current workspace work", 250, 500);

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false))
        .expect("current workspace activity should render");
    assert!(text.contains("current workspace work"), "{text}");
    assert!(!text.contains("<channel name=\"/remote\""), "{text}");
    assert!(!text.contains("old work"), "{text}");
    assert!(!text.contains("state=\"offline\""), "{text}");
    assert!(!text.contains("expired work"), "{text}");
    assert!(!text.contains("self work"), "{text}");
}
