use super::*;
use crate::reconcile::HookContextState;
use crate::state::RecordMessage;

fn record(store: &Store, id: &str, channel: &str, state: &str, created_at: u64) {
    store
        .record_message(&RecordMessage {
            message_id: id.to_string(),
            thread_id: channel.to_string(),
            channel_h: channel.to_string(),
            author_pubkey: OTHER_PK.to_string(),
            body: "hello".to_string(),
            created_at,
            direction: "inbound".to_string(),
            sync_state: state.to_string(),
            native_event_id: Some(id.to_string()),
            error: None,
        })
        .unwrap();
}

#[test]
fn every_channel_shows_only_last_accepted_activity() {
    let store = seed_store();
    store
        .upsert_channel("lounge-h", "lounge", "Lounge", "root", 1)
        .unwrap();
    store
        .replace_channel_members("lounge-h", &[OTHER_PK.into()], 1)
        .unwrap();
    store.replace_channel_admins("lounge-h", &[], 1).unwrap();
    record(&store, "lounge-old", "lounge-h", "accepted", 20);
    record(&store, "lounge-failed", "lounge-h", "failed", 99);
    record(&store, "task-accepted", TASK_H, "accepted", 30);
    let rec = session(&store);

    let xml = render_fabric_context(&store, input(Some(&rec), "root", 0, 140, true)).unwrap();
    assert!(
        xml.contains(
            "<channel name=\"/root/lounge\" about=\"Lounge\" \
             agents=\"1\" last-active=\"2 min ago\" />"
        ),
        "{xml}"
    );
    let task = opening_tag(&xml, "/root/task");
    assert!(task.contains("last-active=\"1 min ago\""), "{task}");
}

#[test]
fn full_and_delta_channels_use_identical_tags_and_nesting() {
    let store = seed_store();
    let rec = session(&store);
    store
        .upsert_channel(TASK_H, "task", "Updated task room", "root", 250)
        .unwrap();
    let captured = capture_inputs(&store, &input(Some(&rec), "root", 0, 300, true)).unwrap();
    let full = render_view_text(&assemble::assemble_view(&captured, 0, 300));
    let delta = render_view_text(&assemble::assemble_view(&captured, 200, 300));

    assert_eq!(
        normalized_opening_tag(&full, "/root"),
        normalized_opening_tag(&delta, "/root")
    );
    assert_eq!(
        normalized_opening_tag(&full, "/root/task"),
        normalized_opening_tag(&delta, "/root/task")
    );
    for xml in [&full, &delta] {
        assert!(
            xml.find("name=\"/root\"").unwrap() < xml.find("name=\"/root/task\"").unwrap(),
            "{xml}"
        );
    }
}

#[test]
fn my_session_full_state_is_byte_identical_to_a_cursor_zero_hook() {
    let store = seed_store();
    let rec = session(&store);
    let (full, _missing) =
        render_full_session_state(&store, &rec, "coder", "", "laptop", 100).expect("full state");
    let captured = capture_inputs(&store, &input(Some(&rec), "root", 0, 100, true)).unwrap();
    let mut hook = HookContextState::default();
    let hook = hook
        .render_context(&rec.pubkey, "turn_start", 0, 100, captured)
        .text
        .expect("cursor-zero hook state");

    assert_eq!(full, hook);
}

#[test]
fn full_rosters_distinguish_humans_from_agents() {
    let store = seed_store();
    store
        .upsert_profile("human", "Pablo", "Pablo", "", false, 1)
        .unwrap();
    store
        .upsert_channel_member("root", "human", "member", 1)
        .unwrap();
    store
        .replace_channel_admins("root", &["unknown-admin".into()], 2)
        .unwrap();
    // Humans never publish heartbeats, so their only trace is what they said.
    human_chat(&store, "human-msg", "root", 40);
    let rec = session(&store);

    let xml = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true)).unwrap();
    assert!(
        xml.contains("<human name=\"@Pablo\" since=\"1 min ago\" />"),
        "{xml}"
    );
    assert!(xml.contains("<agent name=\"@coder\""), "{xml}");
    assert!(
        xml.contains("<channel name=\"/root\" about=\"Root room\" agents=\"2\""),
        "the human row must not inflate the agent-only count: {xml}"
    );
}

#[test]
fn unhydrated_membership_omits_the_agent_count() {
    let store = seed_store();
    let rec = session(&store);
    let mut captured = capture_inputs(&store, &input(Some(&rec), "root", 0, 100, true)).unwrap();
    captured.members.hydrated.remove("root");

    let xml = render_view_text(&assemble::assemble_view(&captured, 0, 100));
    let root = opening_tag(&xml, "/root");
    assert!(!root.contains(" agents="), "{root}");
}

#[test]
fn a_partial_relay_roster_snapshot_never_claims_zero_members() {
    let store = seed_store();
    store
        .upsert_channel("partial-h", "partial", "Partial roster", "root", 1)
        .unwrap();
    store
        .replace_channel_members("partial-h", &[OTHER_PK.into()], 2)
        .unwrap();
    assert_eq!(
        crate::channel_ref::full_channel_ref(&store, "partial-h"),
        "/root/partial"
    );
    let rec = session(&store);

    let partial = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true)).unwrap();
    assert!(partial.contains("/root/partial"), "{partial}");
    let partial_tag = opening_tag(&partial, "/root/partial");
    assert!(!partial_tag.contains(" agents="), "{partial_tag}");

    store.replace_channel_admins("partial-h", &[], 2).unwrap();
    let complete = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true)).unwrap();
    assert!(
        opening_tag(&complete, "/root/partial").contains("agents=\"1\""),
        "{complete}"
    );
}

#[test]
fn hydrated_roster_with_an_unknown_identity_omits_the_agent_count() {
    let store = seed_store();
    store
        .replace_channel_members("root", &[SELF_PK.into(), "unknown-pk".into()], 2)
        .unwrap();
    store.replace_channel_admins("root", &[], 2).unwrap();
    let rec = session(&store);

    let xml = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true)).unwrap();
    let root = opening_tag(&xml, "/root");
    assert!(!root.contains(" agents="), "{root}");
}

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

fn human_chat(store: &crate::state::Store, id: &str, channel: &str, at: u64) {
    store
        .insert_event(&crate::state::RelayEvent {
            id: id.into(),
            kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
            pubkey: "human".into(),
            created_at: at,
            channel_h: channel.into(),
            d_tag: String::new(),
            content: "shipping notes".into(),
            tags_json: "[]".into(),
        })
        .unwrap();
}

fn normalized_opening_tag(xml: &str, id: &str) -> String {
    opening_tag(xml, id).replace(" />", ">")
}

fn opening_tag<'a>(xml: &'a str, id: &str) -> &'a str {
    let needle = format!("name=\"{id}\"");
    let id_at = xml.find(&needle).expect("channel id");
    let start = xml[..id_at].rfind("<channel").expect("channel start");
    let end = xml[id_at..].find('>').expect("channel end") + id_at + 1;
    &xml[start..end]
}

/// A channel whose parent's kind:39000 has not arrived yet is an ordinary
/// transient: you are a member of the child but not of the parent, so the
/// parent row never lands locally. It must not take the whole fabric context
/// down with it — the awareness surface is supposed to degrade, never block.
#[test]
fn a_channel_with_an_unarrived_parent_does_not_sink_the_whole_topology() {
    let store = seed_store();
    store
        .upsert_channel("orphan", "backlog", "", "never-arrived", 1)
        .unwrap();
    store
        .replace_channel_members("orphan", &[SELF_PK.into()], 1)
        .unwrap();

    let rec = session(&store);
    let xml = render_fabric_context(&store, input(Some(&rec), "root", 0, 140, true))
        .expect("an unplaceable channel must not fail the capture");

    // The healthy topology still renders in full...
    assert!(
        xml.contains("<channel name=\"/root\""),
        "root channel missing: {xml}"
    );
    assert!(
        xml.contains("<channel name=\"/root/task\""),
        "task channel missing: {xml}"
    );
    // ...and the unplaceable channel is simply absent, not fatal.
    assert!(
        !xml.contains("backlog"),
        "orphan should be withheld until its ancestry resolves: {xml}"
    );
}
