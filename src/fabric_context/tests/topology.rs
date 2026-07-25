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
fn non_member_channels_show_only_last_accepted_activity() {
    let store = seed_store();
    store
        .upsert_channel("lounge", "lounge", "Lounge", "root", 1)
        .unwrap();
    store
        .replace_channel_members("lounge", &[OTHER_PK.into()], 1)
        .unwrap();
    record(&store, "lounge-old", "lounge", "accepted", 20);
    record(&store, "lounge-failed", "lounge", "failed", 99);
    record(&store, "task-accepted", "task", "accepted", 30);
    let rec = session(&store);

    let xml = render_fabric_context(&store, input(Some(&rec), "root", 0, 140, true)).unwrap();
    assert!(
        xml.contains(
            "<channel name=\"lounge\" id=\"/root/lounge\" about=\"Lounge\" \
             members=\"1\" last-active=\"2 min ago\" />"
        ),
        "{xml}"
    );
    let task = opening_tag(&xml, "/root/task");
    assert!(!task.contains("last-active="), "{task}");
}

#[test]
fn full_and_delta_channels_use_identical_tags_and_nesting() {
    let store = seed_store();
    let rec = session(&store);
    store
        .upsert_channel("task", "task", "Updated task room", "root", 250)
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
            xml.find("id=\"/root\"").unwrap() < xml.find("id=\"/root/task\"").unwrap(),
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
    // Humans never publish heartbeats, so their only trace is what they said.
    human_chat(&store, "human-msg", "root", 40);
    let rec = session(&store);

    let xml = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true)).unwrap();
    assert!(
        xml.contains("<human name=\"@Pablo\" since=\"1 min ago\" />"),
        "{xml}"
    );
    assert!(xml.contains("<agent name=\"@coder\""), "{xml}");
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
    let needle = format!("id=\"{id}\"");
    let id_at = xml.find(&needle).expect("channel id");
    let start = xml[..id_at].rfind("<channel").expect("channel start");
    let end = xml[id_at..].find('>').expect("channel end") + id_at + 1;
    &xml[start..end]
}
