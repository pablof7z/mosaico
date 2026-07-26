use super::*;
use crate::state::{RegisterSession, RelayEvent, Session, Status, Store};
mod agent_about;
mod backend_traffic;
mod channel_tree;
mod cross_workspace;
mod host_profiles;
mod human_render;
mod member_render;
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
fn automatic_context_requires_both_join_fences() {
    let store = seed_store();
    chat(
        &store,
        "future-before-join",
        "root",
        500,
        "future-dated prejoin body",
        "[]",
    );
    let rec = session(&store);
    chat(
        &store,
        "backdated-after-join",
        "root",
        5,
        "backdated postjoin body",
        "[]",
    );
    chat(
        &store,
        "valid-after-join",
        "root",
        210,
        "valid postjoin body",
        "[]",
    );

    let text = render_fabric_context(&store, input(Some(&rec), "root", 0, 600, true))
        .expect("forced context should render");
    assert!(!text.contains("future-dated prejoin body"), "got: {text}");
    assert!(!text.contains("backdated postjoin body"), "got: {text}");
    assert!(text.contains("valid postjoin body"), "got: {text}");
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
    store
        .upsert_reaction(
            "rx-mention-long",
            "mention-long",
            "root",
            SELF_PK,
            "👍",
            211,
        )
        .unwrap();

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false))
        .expect("mention should render");
    assert!(text.contains("<channels>"));
    assert!(!text.contains("<workspace"));
    assert!(text.contains("<channel name=\"/root\""));
    assert!(!text.contains("<channel name=\"/root\" id=\""));
    assert!(text.contains("<message from=\"@reviewer\" id=\"mentio\">"));
    assert!(
        !text.contains("Need a follow-up? Read `skills/mosaico/references/coordination-guide.md`."),
    );
    assert!(!text.contains("mention=\"true\""));
    assert!(!text.contains("truncated=\"true\""));
    assert!(text.contains("<important>"));
    assert!(text.contains("<mention channel=\"/root\""));
    assert!(text.contains("message_id=\"mentio\""));
}

#[test]
fn mention_rows_without_followup_show_coordination_guide_nudge() {
    let store = seed_store();
    let rec = session(&store);
    let tags = format!("[[\"p\",\"{SELF_PK}\"]]");
    chat(
        &store,
        "mention-guide",
        "root",
        210,
        "please review this",
        &tags,
    );

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false))
        .expect("mention should render");

    assert!(
        text.contains("Need a follow-up? Read `skills/mosaico/references/coordination-guide.md`."),
        "got: {text}"
    );
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
    use nostr::{PublicKey, ToBech32};

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
