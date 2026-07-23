use super::*;
use crate::state::{RegisterSession, RelayEvent, Session, Status, Store};
mod agent_about;
mod backend_traffic;
mod channel_tree;
mod cross_workspace;
mod member_render;
mod reactions;
mod session_title;

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
    store.grant_session_route(&rec.pubkey, "task", 20).unwrap();
    rec
}

fn session_record(store: &Store, _label: &str, channel_h: &str) -> Session {
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: SELF_PK.into(),
            observed_harness: "codex".into(),
            agent_slug: "coder".into(),
            channel_h: channel_h.into(),
            child_pid: None,
            transcript_path: None,
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
fn human_who_renderer_is_non_xml_and_terminal_friendly() {
    let store = seed_store();
    publish_idle_status(&store, SELF_PK, "coder", "Reviewing fabric context");

    let human = render_fabric_context_human(&store, input(None, "root", 0, 1_000, true), false)
        .expect("valid channel ancestry")
        .expect("human who should render");

    assert!(human.starts_with("root\nRoot room\n\n"), "got: {human}");
    assert!(human.contains("#root.task"), "got: {human}");
    assert!(human.contains("Members"), "got: {human}");
    assert!(human.contains("@coder"), "got: {human}");
    assert!(human.contains("idle"), "got: {human}");
    assert!(!human.contains(" member "), "got: {human}");
    assert!(!human.contains(" admin "), "got: {human}");
    assert!(!human.contains("<mosaico>"), "got: {human}");
    assert!(!human.contains("<member"), "got: {human}");
}

#[test]
fn human_who_renderer_colorizes_when_requested() {
    let store = seed_store();
    publish_idle_status(&store, SELF_PK, "coder", "Reviewing fabric context");

    let human = render_fabric_context_human(&store, input(None, "root", 0, 1_000, true), true)
        .expect("valid channel ancestry")
        .expect("human who should render");

    assert!(
        human.contains("\u{1b}["),
        "expected ansi styling: {human:?}"
    );
    assert!(human.contains("@coder"), "got: {human}");
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
    assert!(text.contains("<workspace name=\"root\" channel=\"root\""));
    assert!(!text.contains("<channel name=\"#root\""));
    assert!(text.contains("<message from=\"@reviewer\" id=\"mentio\">"));
    assert!(text.contains("Reply via: `mosaico channel reply mentio --message \"hello world\"`"));
    assert!(text.contains("Attachments: add `--attach label=/path/to/file`"));
    assert!(!text.contains("mention=\"true\""));
    assert!(!text.contains("truncated=\"true\""));
    assert!(text.contains("<important>"));
    assert!(text.contains("<mention channel=\"root\""));
    assert!(text.contains("message_id=\"mentio\""));
}

#[test]
fn injected_mention_row_is_hidden_from_chatter() {
    let store = seed_store();
    let rec = session(&store);
    let tags = format!("[[\"p\",\"{SELF_PK}\"]]");
    chat(
        &store,
        "mention-inj",
        "root",
        210,
        "please pick this up",
        &tags,
    );

    store
        .enqueue_inbox(
            "mention-inj",
            &rec.pubkey,
            OTHER_PK,
            "root",
            "please pick this up",
            210,
        )
        .unwrap();
    store.claim_pending_for_pubkey(&rec.pubkey, 210).unwrap();
    store
        .mark_injected_for_echo(&["mention-inj".to_string()], &rec.pubkey, 210)
        .unwrap();

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, true))
        .expect("forced context should still render");
    assert!(!text.contains("please pick this up"));
}

#[test]
fn message_rows_show_p_tag_recipients_and_rewrite_nostr_mentions() {
    use nostr_sdk::prelude::{PublicKey, ToBech32};

    const TARGET_PK: &str = "379e863e8357163b5bce5d2688dc4f1dcc2d505222fb8d74db600f30535dfdfe";
    const REMOTE_PK: &str = "9aa6883eee2f1ce43053a1eec2c1c8b1c712cbb3c77ec346d9f091982a50b461";

    let store = seed_store();
    let rec = session(&store);
    store
        .upsert_profile(TARGET_PK, "target@laptop", "target", "laptop", false, 1)
        .unwrap();
    store
        .upsert_profile(REMOTE_PK, "remote@tower", "remote", "tower", false, 1)
        .unwrap();
    let npub = PublicKey::from_hex(TARGET_PK).unwrap().to_bech32().unwrap();
    let tags = format!("[[\"p\",\"{TARGET_PK}\"],[\"p\",\"{REMOTE_PK}\"]]");
    chat(
        &store,
        "mention-target",
        "root",
        210,
        &format!("please ask nostr:{npub} for review"),
        &tags,
    );

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false))
        .expect("p-tagged ambient message should render");
    assert!(
        text.contains("for=\"@target @remote@tower\""),
        "got: {text}"
    );
    assert!(text.contains("please ask @target@laptop for review"));
    assert!(!text.contains("nostr:npub"), "got: {text}");

    let captured = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    let rendered = render_view_text(&assemble::assemble_view(&captured, 200, 300));
    assert_eq!(rendered, text);
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
    assert!(forced.contains("Agent: coder · Session: @coder · Backend: laptop"));
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

    let captured = capture_inputs(&store, &input(Some(&rec), "ghost", 0, 100, false)).unwrap();
    let rendered = render_view_text(&assemble::assemble_view(&captured, 0, 100));
    assert_eq!(rendered, direct);
}

#[test]
fn all_workspaces_agent_context_omits_rosters_while_human_view_preserves_them() {
    let store = seed_store();
    store
        .upsert_channel("other", "other", "Other workspace", "", 1)
        .unwrap();
    store
        .replace_agent_roster(&crate::state::AgentRoster {
            backend_pubkey: "backend".into(),
            host: "laptop".into(),
            slug: "shared".into(),
            use_criteria: "Available everywhere".into(),
            channels: vec!["root".into(), "other".into()],
            updated_at: 2,
        })
        .unwrap();
    store
        .replace_agent_roster(&crate::state::AgentRoster {
            backend_pubkey: "backend".into(),
            host: "laptop".into(),
            slug: "other-only".into(),
            use_criteria: "Only in other".into(),
            channels: vec!["other".into()],
            updated_at: 2,
        })
        .unwrap();

    let roots = vec!["root".into(), "other".into()];
    let rendered = render_fabric_all_workspaces(&store, &roots, 100, "laptop", "");
    assert_eq!(rendered.matches("<mosaico>").count(), 1, "got: {rendered}");
    assert!(!rendered.contains("mosaico agents list"), "got: {rendered}");
    assert!(!rendered.contains("<available-agents>"), "got: {rendered}");
    assert!(!rendered.contains("<workspace-agents>"), "got: {rendered}");
    assert!(!rendered.contains("@shared"), "got: {rendered}");
    assert!(!rendered.contains("@other-only"), "got: {rendered}");

    let human =
        render_fabric_all_workspaces_human(&store, &roots, 100, "laptop", "", false).unwrap();
    assert_eq!(
        human.matches("Available agents (all workspaces)").count(),
        1,
        "got: {human}"
    );
    assert_eq!(human.matches("@shared").count(), 1, "got: {human}");
    assert_eq!(
        human.matches("Workspace-specific agents").count(),
        1,
        "got: {human}"
    );
    assert!(human.contains("@other-only"), "got: {human}");
}

/// A forced but empty delta (nothing new since the cursor) must explain that the
/// fabric reports only changes, NOT emit a bare empty `<workspace>` skeleton that
/// reads as "channels disappeared". Regression for the confusing second `who`.
#[test]
fn quiet_forced_delta_renders_no_new_activity_note() {
    let store = seed_store();
    let rec = session(&store);

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, true))
        .expect("forced who should always render");
    assert!(text.contains("Agent: coder · Session: @coder · Backend: laptop"));
    assert!(text.contains("<no-new-activity workspace=\"root\">"));
    assert!(text.contains("The fabric surfaces only what changed"));
    // The tell-tale empty skeleton must NOT appear: no channel/members blocks.
    assert!(!text.contains("<members>"), "got: {text}");
    assert!(!text.contains("<channel name="), "got: {text}");

    // Parity: the pure capture→assemble path renders identically.
    let captured = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, true)).unwrap();
    let rendered = render_view_text(&assemble::assemble_view(&captured, 200, 300));
    assert_eq!(rendered, text);
}
