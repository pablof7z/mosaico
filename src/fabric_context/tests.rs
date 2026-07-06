use super::*;
use crate::state::{RegisterSession, RelayEvent, Session, Status, Store};

const SELF_PK: &str = "self-pubkey";
const OTHER_PK: &str = "other-pubkey";

fn seed_store() -> Store {
    let store = Store::open_memory().unwrap();
    store
        .upsert_channel("root", "main", "Root room", "", 1)
        .unwrap();
    store
        .upsert_channel("task", "task", "Task room", "root", 1)
        .unwrap();
    store
        .replace_channel_members("root", &[SELF_PK.into(), OTHER_PK.into()], 1)
        .unwrap();
    store
        .replace_channel_members("task", &[SELF_PK.into(), OTHER_PK.into()], 1)
        .unwrap();
    store
        .upsert_profile(SELF_PK, "coder", "coder", "laptop", false, 1)
        .unwrap();
    store
        .upsert_profile(OTHER_PK, "reviewer", "reviewer", "laptop", false, 1)
        .unwrap();
    store
}

fn session(store: &Store) -> Session {
    let rec = session_record(store, "sess", "root");
    store
        .join_session_channel(&rec.session_id, "task", 20)
        .unwrap();
    rec
}

fn session_record(store: &Store, external_id: &str, channel_h: &str) -> Session {
    let id = store
        .register_session(&RegisterSession {
            harness: "test".into(),
            external_id_kind: "test".into(),
            external_id: external_id.into(),
            agent_pubkey: SELF_PK.into(),
            agent_slug: "coder".into(),
            channel_h: channel_h.into(),
            child_pid: None,
            transcript_path: None,
            resume_id: String::new(),
            now: 10,
        })
        .unwrap();
    store.get_session(&id).unwrap().unwrap()
}

fn chat(store: &Store, id: &str, channel: &str, at: u64, body: &str, tags_json: &str) {
    store
        .insert_event(&RelayEvent {
            id: id.into(),
            kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
            pubkey: OTHER_PK.into(),
            created_at: at,
            channel_h: channel.into(),
            d_tag: String::new(),
            content: body.into(),
            tags_json: tags_json.into(),
        })
        .unwrap();
}

fn input<'a>(
    rec: Option<&'a Session>,
    scope: &'a str,
    cursor: u64,
    now: u64,
    force: bool,
) -> FabricContextInput<'a> {
    FabricContextInput {
        session: rec,
        scope,
        cursor,
        now,
        self_slug: "coder",
        self_pubkey: SELF_PK,
        local_host: "laptop",
        forced_messages: &[],
        warnings: &[],
        force,
    }
}

#[test]
fn session_view_has_self_and_chatter_human_view_does_not() {
    let store = seed_store();
    let rec = session(&store);
    chat(&store, "m1", "root", 900, "post join context", "[]");

    let agent = render_fabric_context(&store, input(Some(&rec), "root", 0, 1_000, false))
        .expect("session view should render");
    assert!(agent.contains("You are @coder on laptop"));
    assert!(agent.contains("<chatter>"));
    assert!(agent.contains("post join context"));
    assert!(agent.contains("<subchannels>"));

    let human = render_fabric_context(&store, input(None, "root", 0, 1_000, true))
        .expect("human who should render");
    assert!(human.contains("<tenex-edge>"));
    assert!(!human.contains("You are @"));
    assert!(!human.contains("<chatter>"));
}

#[test]
fn human_who_renderer_is_non_xml_and_terminal_friendly() {
    let store = seed_store();

    let human = render_fabric_context_human(&store, input(None, "root", 0, 1_000, true), false)
        .expect("human who should render");

    assert!(human.starts_with("main\nRoot room\n\n"), "got: {human}");
    assert!(human.contains("#main"), "got: {human}");
    assert!(human.contains("Members"), "got: {human}");
    assert!(human.contains("@coder"), "got: {human}");
    assert!(human.contains("offline"), "got: {human}");
    assert!(!human.contains("<tenex-edge>"), "got: {human}");
    assert!(!human.contains("<member"), "got: {human}");
}

#[test]
fn human_who_renderer_colorizes_when_requested() {
    let store = seed_store();

    let human = render_fabric_context_human(&store, input(None, "root", 0, 1_000, true), true)
        .expect("human who should render");

    assert!(
        human.contains("\u{1b}["),
        "expected ansi styling: {human:?}"
    );
    assert!(human.contains("@coder"), "got: {human}");
}

#[test]
fn cursor_delta_only_renders_changed_joined_channel() {
    let store = seed_store();
    let rec = session(&store);
    chat(&store, "old-root", "root", 100, "old root message", "[]");
    chat(&store, "new-task", "task", 220, "new task message", "[]");

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false))
        .expect("changed task channel should render");
    assert!(text.contains("name=\"#task\""));
    assert!(text.contains("new task message"));
    assert!(!text.contains("name=\"#main\""));
    assert!(!text.contains("old root message"));
}

#[test]
fn archived_joined_channels_are_hidden_from_fabric_context() {
    let store = seed_store();
    let rec = session(&store);
    store
        .upsert_channel("archived", "archived", "[ARCHIVED] done", "root", 30)
        .unwrap();
    store
        .join_session_channel(&rec.session_id, "archived", 30)
        .unwrap();
    chat(
        &store,
        "archived-chat",
        "archived",
        220,
        "old task note",
        "[]",
    );

    let text = render_fabric_context(&store, input(Some(&rec), "root", 0, 300, true))
        .expect("forced context should render");
    assert!(!text.contains("name=\"#archived\""));
    assert!(!text.contains("[ARCHIVED] done"));
    assert!(!text.contains("old task note"));
}

#[test]
fn mention_rows_are_marked_important_and_truncated_with_recovery_id() {
    let store = seed_store();
    let rec = session(&store);
    let body = (0..305)
        .map(|i| format!("word{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let tags = format!("[[\"p\",\"{SELF_PK}\"]]");
    chat(&store, "mention-long", "root", 210, &body, &tags);

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false))
        .expect("mention should render");
    assert!(text.contains("[MENTIONS YOU]"));
    assert!(!text.contains("mention=\"true\""));
    assert!(!text.contains("truncated=\"true\""));
    assert!(text.contains("<important>"));
    assert!(text.contains("message_id=\"mentio\""));
    assert!(text.contains("tenex-edge chat read --id mentio"));
}

#[test]
fn empty_delta_is_silent_unless_forced() {
    let store = seed_store();
    let rec = session(&store);

    let quiet = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false));
    assert!(
        quiet.is_none(),
        "empty hook delta should be silent: {quiet:?}"
    );

    let forced = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, true))
        .expect("explicit who context should still render");
    assert!(forced.contains("You are @coder on laptop"));
}

#[test]
fn recent_presence_uses_status_source() {
    let store = seed_store();
    let rec = session(&store);
    store
        .upsert_status(&Status {
            pubkey: OTHER_PK.into(),
            session_id: "other-session".into(),
            channel_h: "root".into(),
            slug: "reviewer".into(),
            title: "Reviewing".into(),
            activity: "checking tests".into(),
            busy: true,
            last_seen: 250,
            updated_at: 250,
            expiration: 500,
        })
        .unwrap();

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false))
        .expect("presence delta should render");
    assert!(text.contains("<recent-presence>"));
    assert!(text.contains("ref=\"@reviewer\""));
    assert!(text.contains("text=\"checking tests\""));
}

#[test]
fn missing_channels_are_warned_not_rendered() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_profile(SELF_PK, "coder", "coder", "laptop", false, 1)
        .unwrap();
    let rec = session_record(&store, "missing", "ghost");

    let direct = render_fabric_context(&store, input(Some(&rec), "ghost", 0, 100, false))
        .expect("missing channel warning should render");
    assert!(direct.contains("Fabric channel \"ghost\" is unavailable"));
    assert!(!direct.contains("<channel name=\"#ghost\""));
    assert!(!direct.contains("<members>"));

    let captured = capture_inputs(&store, &input(Some(&rec), "ghost", 0, 100, false));
    let trellis = render_view_text(&assemble::assemble_view(&captured, 0, 100));
    assert_eq!(trellis, direct);
}

#[test]
fn members_are_relay_roster_backed_and_local_agents_are_labeled() {
    let store = seed_store();
    let rec = session(&store);
    store
        .replace_agent_roster(&crate::state::AgentRoster {
            backend_pubkey: "backend".into(),
            host: "laptop".into(),
            slug: "helper".into(),
            use_criteria: "For testing".into(),
            channels: vec!["root".into()],
            updated_at: 2,
        })
        .unwrap();

    let text = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true))
        .expect("context should render");
    assert!(text.contains("<available-agents>"));
    assert!(text.contains("<agent ref=\"@helper\" about=\"For testing\""));
    assert!(!text.contains("<agents>"));
    assert!(text.contains("<member ref=\"@coder\""));

    let empty = Store::open_memory().unwrap();
    empty.upsert_channel("solo", "solo", "", "", 1).unwrap();
    empty
        .upsert_profile(SELF_PK, "coder", "coder", "laptop", false, 1)
        .unwrap();
    let solo = session_record(&empty, "solo", "solo");
    let text = render_fabric_context(&empty, input(Some(&solo), "solo", 0, 100, true)).unwrap();
    assert!(text.contains("<channel name=\"#solo\""));
    assert!(!text.contains("<members>"), "got: {text}");
}
