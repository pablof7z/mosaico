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
    let id = store
        .register_session(&RegisterSession {
            harness: "test".into(),
            external_id_kind: "test".into(),
            external_id: "sess".into(),
            agent_pubkey: SELF_PK.into(),
            agent_slug: "coder".into(),
            channel_h: "root".into(),
            child_pid: None,
            transcript_path: None,
            resume_id: String::new(),
            now: 10,
        })
        .unwrap();
    store.join_session_channel(&id, "task", 20).unwrap();
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
        edge_home: None,
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
    assert!(agent.contains("<self agent=\"@coder\""));
    assert!(agent.contains("<chatter>"));
    assert!(agent.contains("post join context"));
    assert!(agent.contains("<subchannels>"));

    let human = render_fabric_context(&store, input(None, "root", 0, 1_000, true))
        .expect("human who should render");
    assert!(human.contains("<tenex-edge>"));
    assert!(!human.contains("<self "));
    assert!(!human.contains("<chatter>"));
}

#[test]
fn cursor_delta_only_renders_changed_joined_channel() {
    let store = seed_store();
    let rec = session(&store);
    chat(&store, "old-root", "root", 100, "old root message", "[]");
    chat(&store, "new-task", "task", 220, "new task message", "[]");

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false))
        .expect("changed task channel should render");
    assert!(text.contains("id=\"task\""));
    assert!(text.contains("new task message"));
    assert!(!text.contains("id=\"root\""));
    assert!(!text.contains("old root message"));
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
    assert!(text.contains("mention=\"true\""));
    assert!(text.contains("truncated=\"true\""));
    assert!(text.contains("<important>"));
    assert!(text.contains("message_id=\"mention-long\""));
    assert!(text.contains("tenex-edge chat read --id mention-long"));
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
    assert!(forced.contains("<self agent=\"@coder\""));
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
