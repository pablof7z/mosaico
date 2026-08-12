use super::*;
use crate::state::{
    Profile, RegisterSession, RelayEvent, Session, Status, Store, TestGroup, TestGroupDelivery,
    TestRelayDelivery,
};
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
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([root_group(), task_group()]));
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().profiles(seed_profiles()));
    store
}

fn seed_profiles() -> Vec<Profile> {
    vec![
        profile(SELF_PK, "coder", "coder", "coder", "laptop", false),
        profile(
            OTHER_PK, "reviewer", "reviewer", "reviewer", "laptop", false,
        ),
    ]
}

fn profile(
    pubkey: &str,
    name: &str,
    slug: &str,
    agent_slug: &str,
    host: &str,
    is_backend: bool,
) -> Profile {
    Profile {
        pubkey: pubkey.into(),
        name: name.into(),
        slug: slug.into(),
        agent_slug: agent_slug.into(),
        host: host.into(),
        is_backend,
        agents: Vec::new(),
        workspaces: Vec::new(),
        updated_at: 1,
    }
}

fn pubkeys(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn root_group() -> TestGroup {
    root_group_with_roster(&[SELF_PK, OTHER_PK], &[])
}

fn root_group_with_roster(members: &[&str], admins: &[&str]) -> TestGroup {
    TestGroup::new("root")
        .metadata("main", "Root room", "", 1)
        .admins(pubkeys(admins))
        .members(pubkeys(members))
}

fn task_group() -> TestGroup {
    task_group_with_roster(&[SELF_PK, OTHER_PK], &[])
}

fn task_group_with_metadata(about: &str, as_of: u64) -> TestGroup {
    TestGroup::new(TASK_H)
        .metadata("task", about, "root", as_of)
        .admins(Vec::new())
        .members(pubkeys(&[SELF_PK, OTHER_PK]))
}

fn task_group_with_roster(members: &[&str], admins: &[&str]) -> TestGroup {
    TestGroup::new(TASK_H)
        .metadata("task", "Task room", "root", 1)
        .admins(pubkeys(admins))
        .members(pubkeys(members))
}

fn idle_status(pubkey: &str, slug: &str, title: &str) -> Status {
    Status {
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
    }
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

fn chat(id: &str, channel: &str, at: u64, body: &str, tags_json: &str) -> RelayEvent {
    RelayEvent {
        id: id.into(),
        kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
        pubkey: OTHER_PK.into(),
        created_at: at,
        channel_h: channel.into(),
        d_tag: String::new(),
        content: body.into(),
        tags_json: tags_json.into(),
    }
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
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([
        root_group(),
        task_group(),
        TestGroup::new("archived").metadata("archived", "[ARCHIVED] done", "root", 30),
    ]));
    store
        .grant_session_route(&rec.pubkey, "archived", 30)
        .unwrap();
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .events([chat(
                "archived-chat",
                "archived",
                220,
                "old task note",
                "[]",
            )]),
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
    assert!(
        forced.contains("<self name=\"@coder\" host=\"laptop\" headless=\"off\" unhosted=\"true\"")
    );
}

#[test]
fn self_unhosted_fact_depends_on_admitted_transport_not_endpoint_liveness() {
    let store = seed_store();
    let unhosted = session(&store);
    let unhosted_text =
        render_fabric_context(&store, input(Some(&unhosted), "root", 0, 300, true)).unwrap();
    assert!(unhosted_text.contains("unhosted=\"true\""));

    let mut hosted_without_locator = unhosted;
    hosted_without_locator.admitted_transport = "pty".into();
    let hosted_text = render_fabric_context(
        &store,
        input(Some(&hosted_without_locator), "root", 0, 300, true),
    )
    .unwrap();
    assert!(!hosted_text.contains("unhosted=\"true\""));
}

#[test]
fn missing_channels_are_warned_not_rendered() {
    let store = Store::open_memory().unwrap();
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles([profile(SELF_PK, "coder", "coder", "", "laptop", false)]),
    );
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
    assert!(
        text.contains("<self name=\"@coder\" host=\"laptop\" headless=\"off\" unhosted=\"true\"")
    );
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
