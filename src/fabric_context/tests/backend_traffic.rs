use crate::fabric_context::{
    assemble, capture_inputs, render_fabric_context, render_view_text, FabricContextInput,
};
use crate::state::{RelayEvent, Store};

use super::{publish_idle_status, seed_store, session, OTHER_PK, SELF_PK};

/// The daemon's own management pubkey is excluded from the roster by identity,
/// even when it has no cached kind:0 profile.
#[test]
fn backend_pubkey_excluded_from_roster_without_cached_profile() {
    const MGMT_PK: &str = "backend-mgmt-pubkey";
    let store = seed_store();
    store
        .replace_channel_members(
            "root",
            &[SELF_PK.into(), OTHER_PK.into(), MGMT_PK.into()],
            2,
        )
        .unwrap();
    publish_idle_status(&store, SELF_PK, "coder", "Coding");
    publish_idle_status(&store, OTHER_PK, "reviewer", "Reviewing");
    publish_idle_status(&store, MGMT_PK, "backend", "Managing relay");
    let rec = session(&store);

    let excluded = |backend_pubkey: &str| -> String {
        let build = FabricContextInput {
            session: Some(&rec),
            scope: "root",
            cursor: 0,
            now: 100,
            self_slug: "coder",
            self_pubkey: SELF_PK,
            backend_pubkey,
            local_host: "laptop",
            forced_messages: &[],
            warnings: &[],
            force: true,
        };
        let text = render_fabric_context(&store, build).expect("roster should render");
        let cap_input = FabricContextInput {
            session: Some(&rec),
            scope: "root",
            cursor: 0,
            now: 100,
            self_slug: "coder",
            self_pubkey: SELF_PK,
            backend_pubkey,
            local_host: "laptop",
            forced_messages: &[],
            warnings: &[],
            force: true,
        };
        let captured = capture_inputs(&store, &cap_input).unwrap();
        let rendered = render_view_text(&assemble::assemble_view(&captured, 0, 100));
        assert_eq!(rendered, text, "captured rendering must be deterministic");
        text
    };

    let filtered = excluded(MGMT_PK);
    assert!(filtered.contains("<member ref=\"@coder\""));
    assert!(filtered.contains("<member ref=\"@reviewer\""));
    assert!(
        !filtered.contains("@backend"),
        "mgmt key leaked into roster: {filtered}"
    );

    let leaked = excluded("");
    assert!(
        leaked.contains("@backend"),
        "control: mgmt key should leak when its identity is unknown: {leaked}"
    );
}

/// Backend-to-party traffic is excluded from ambient chatter when the author or
/// a directed p-tag recipient is a backend. Regression for #276.
#[test]
fn backend_traffic_excluded_from_chatter() {
    const MGMT_PK: &str = "backend-mgmt-pubkey";
    const REMOTE_BACKEND_PK: &str = "remote-backend-pubkey";
    let store = seed_store();
    store
        .upsert_profile(REMOTE_BACKEND_PK, "hub", "hub", "tower", true, 1)
        .unwrap();
    let rec = session(&store);

    chat_from(
        &store,
        "human-msg",
        "root",
        OTHER_PK,
        900,
        "normal chatter",
        "[]",
    );
    chat_from(
        &store,
        "mgmt-msg",
        "root",
        MGMT_PK,
        910,
        "backend announcement",
        "[]",
    );
    chat_from(
        &store,
        "remote-msg",
        "root",
        REMOTE_BACKEND_PK,
        920,
        "remote backend note",
        "[]",
    );
    chat_from(
        &store,
        "to-mgmt",
        "root",
        OTHER_PK,
        930,
        "hey daemon",
        &format!("[[\"p\",\"{MGMT_PK}\"]]"),
    );

    let render = |backend_pubkey: &str| -> String {
        let build = FabricContextInput {
            session: Some(&rec),
            scope: "root",
            cursor: 0,
            now: 1_000,
            self_slug: "coder",
            self_pubkey: SELF_PK,
            backend_pubkey,
            local_host: "laptop",
            forced_messages: &[],
            warnings: &[],
            force: false,
        };
        let text = render_fabric_context(&store, build).expect("chatter should render");
        let cap_input = FabricContextInput {
            session: Some(&rec),
            scope: "root",
            cursor: 0,
            now: 1_000,
            self_slug: "coder",
            self_pubkey: SELF_PK,
            backend_pubkey,
            local_host: "laptop",
            forced_messages: &[],
            warnings: &[],
            force: false,
        };
        let captured = capture_inputs(&store, &cap_input).unwrap();
        let rendered = render_view_text(&assemble::assemble_view(&captured, 0, 1_000));
        assert_eq!(rendered, text, "captured rendering must be deterministic");
        text
    };

    let text = render(MGMT_PK);
    assert!(
        text.contains("normal chatter"),
        "human chatter must stay: {text}"
    );
    assert!(
        !text.contains("backend announcement"),
        "backend-authored identity traffic leaked: {text}"
    );
    assert!(
        !text.contains("remote backend note"),
        "backend-authored profile traffic leaked: {text}"
    );
    assert!(
        !text.contains("hey daemon"),
        "backend-directed traffic leaked: {text}"
    );

    let leaked = render("");
    assert!(
        leaked.contains("backend announcement"),
        "control: mgmt-authored should leak without identity: {leaked}"
    );
    assert!(
        leaked.contains("hey daemon"),
        "control: mgmt-directed should leak without identity: {leaked}"
    );
    assert!(
        !leaked.contains("remote backend note"),
        "is_backend author should stay hidden without identity: {leaked}"
    );
}

fn chat_from(
    store: &Store,
    id: &str,
    channel: &str,
    pubkey: &str,
    at: u64,
    body: &str,
    tags_json: &str,
) {
    store
        .insert_event(&RelayEvent {
            id: id.into(),
            kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
            pubkey: pubkey.into(),
            created_at: at,
            channel_h: channel.into(),
            d_tag: String::new(),
            content: body.into(),
            tags_json: tags_json.into(),
        })
        .unwrap();
}
