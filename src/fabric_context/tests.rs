use super::*;
use crate::state::{RegisterSession, RelayEvent, Session, Status, Store};
mod agent_about;
mod backend_traffic;
mod channel_tree;
mod cross_workspace;
mod host_profiles;
mod human_render;
mod member_render;
mod message_context;
mod reactions;
mod roster_awareness;
mod session_title;
mod topology;

const SELF_PK: &str = "self-pubkey";
const OTHER_PK: &str = "other-pubkey";
const TASK_H: &str = "task-h";

fn seed_store() -> Store {
    let store = Store::open_memory().unwrap();
    store
        .upsert_channel("root", "main", "Root room", "", 1)
        .unwrap();
    store
        .upsert_channel(TASK_H, "task", "Task room", "root", 1)
        .unwrap();
    store
        .replace_channel_members("root", &[SELF_PK.into(), OTHER_PK.into()], 1)
        .unwrap();
    store.replace_channel_admins("root", &[], 1).unwrap();
    store
        .replace_channel_members(TASK_H, &[SELF_PK.into(), OTHER_PK.into()], 1)
        .unwrap();
    store.replace_channel_admins(TASK_H, &[], 1).unwrap();
    for (pk, slug) in [(SELF_PK, "coder"), (OTHER_PK, "reviewer")] {
        store
            .upsert_profile_with_agent_slug(pk, slug, slug, slug, "laptop", false, 1)
            .unwrap();
    }
    store
}

fn publish_idle_status(store: &Store, pubkey: &str, slug: &str, title: &str) {
    store
        .upsert_status(&Status {
            pubkey: pubkey.into(),
            channel_h: "root".into(),
            slug: slug.into(),
            title: title.into(),
            activity: String::new(),
            workspace: "root".into(),
            branch: String::new(),
            state: crate::session_state::SessionState::Idle,
            state_since: 90,
            last_seen: 90,
            updated_at: 90,
            expiration: 2_000,
        })
        .unwrap();
}

fn session(store: &Store) -> Session {
    let rec = session_record(store, "sess", "root");
    store.grant_session_route(&rec.pubkey, TASK_H, 20).unwrap();
    rec
}

fn session_record(store: &Store, _label: &str, channel_h: &str) -> Session {
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: SELF_PK.into(),
            observed_harness: "codex".into(),
            agent_slug: "coder".into(),
            launch_channel_h: channel_h.into(),
            work_root: channel_h.split('-').next().unwrap_or(channel_h).to_string(),
            child_pid: None,
            now: 10,
        })
        .unwrap();
    store.get_session(SELF_PK).unwrap().unwrap()
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
        backend_pubkey: "",
        local_host: "laptop",
        forced_messages: &[],
        warnings: &[],
        force,
    }
}

#[test]
fn archived_joined_channels_are_hidden_from_fabric_context() {
    let store = seed_store();
    let rec = session(&store);
    store
        .upsert_channel("archived", "archived", "[ARCHIVED] done", "root", 30)
        .unwrap();
    store
        .grant_session_route(&rec.pubkey, "archived", 30)
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
    assert!(!text.contains("name=\"archived\""));
    assert!(!text.contains("[ARCHIVED] done"));
    assert!(!text.contains("old task note"));
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
    assert!(forced.contains("<self name=\"@coder\" host=\"laptop\" headless=\"off\""));
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
    assert!(!direct.contains("<channel name=\"ghost\""));
    assert!(!direct.contains("<members>"));

    let captured = capture_inputs(&store, &input(Some(&rec), "ghost", 0, 100, false)).unwrap();
    let rendered = render_view_text(&assemble::assemble_view(&captured, 0, 100));
    assert_eq!(rendered, direct);
}

/// A forced but empty delta (nothing new since the cursor) must explain that the
/// fabric reports only changes, NOT emit a bare empty `<channels>` skeleton that
/// reads as "channels disappeared". Regression for the confusing second `who`.
#[test]
fn quiet_forced_delta_renders_no_new_activity_note() {
    let store = seed_store();
    let rec = session(&store);

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, true))
        .expect("forced who should always render");
    assert!(text.contains("<self name=\"@coder\" host=\"laptop\" headless=\"off\""));
    assert!(text.contains("<no-new-activity workspace=\"root\">"));
    assert!(text.contains("The fabric surfaces only what changed"));
    // The tell-tale empty skeleton must NOT appear: no channel/members blocks.
    assert!(!text.contains("<members>"), "got: {text}");
    assert!(!text.contains("<channel name="), "got: {text}");
    assert!(!text.contains("<channels>"), "got: {text}");

    // Parity: the pure capture→assemble path renders identically.
    let captured = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, true)).unwrap();
    let rendered = render_view_text(&assemble::assemble_view(&captured, 200, 300));
    assert_eq!(rendered, text);
}
