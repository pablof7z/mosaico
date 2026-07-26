use super::*;
use crate::reconcile::HookContextState;
use crate::state::RecordMessage;

mod hook_cache;
mod member_deltas;

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

    assert!(
        xml.contains("<channel name=\"/root\""),
        "root channel missing: {xml}"
    );
    assert!(
        xml.contains("<channel name=\"/root/task\""),
        "task channel missing: {xml}"
    );
    assert!(
        !xml.contains("backlog"),
        "orphan should be withheld until its ancestry resolves: {xml}"
    );
}
