use super::*;
use crate::state::{
    Profile, RegisterSession, RelayEvent, TestGroup, TestGroupDelivery, TestRelayDelivery,
};

fn profile(pubkey: &str, name: &str, agent_slug: &str, is_backend: bool) -> Profile {
    Profile {
        pubkey: pubkey.into(),
        name: name.into(),
        slug: name.into(),
        agent_slug: agent_slug.into(),
        host: "host".into(),
        is_backend,
        agents: Vec::new(),
        workspaces: Vec::new(),
        updated_at: 1,
    }
}

fn message(
    id: &str,
    channel: &str,
    author: &str,
    body: &str,
    at: u64,
    recipients: &[&str],
) -> RelayEvent {
    RelayEvent {
        id: id.into(),
        kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
        pubkey: author.into(),
        created_at: at,
        channel_h: channel.into(),
        d_tag: String::new(),
        content: body.into(),
        tags_json: serde_json::to_string(
            &recipients
                .iter()
                .map(|pk| vec!["p", *pk])
                .collect::<Vec<_>>(),
        )
        .unwrap(),
    }
}

#[tokio::test]
async fn projects_only_exact_channels_and_directly_interacted_kind_zero_agents() {
    let state = DaemonState::new_for_test().await;
    let now = crate::util::now_secs();
    state
        .with_store(|store| {
            store.install_test_group_delivery(TestGroupDelivery::new([
                TestGroup::new("root").metadata("root", "Main coordination", "", 1),
                TestGroup::new("joined-h").metadata("joined", "Panel tests", "root", 2),
                TestGroup::new("sibling-h").metadata("sibling", "", "root", 2),
            ]));
            store.install_test_relay_delivery(
                TestRelayDelivery::new()
                    .profiles([
                        profile("self", "drift-pi", "pi", false),
                        profile("in", "haven-codex", "codex", false),
                        profile("out", "bravo-codex", "codex", false),
                        profile("chatter", "other-codex", "codex", false),
                        profile("human", "pablo", "", false),
                        profile("backend", "mosaico", "", true),
                    ])
                    .events([
                        message(
                            "incoming",
                            "root",
                            "in",
                            "latest from haven",
                            now - 20,
                            &["self"],
                        ),
                        message(
                            "older",
                            "root",
                            "in",
                            "older from haven",
                            now - 25,
                            &["self"],
                        ),
                        message(
                            "outgoing",
                            "joined-h",
                            "self",
                            "latest to bravo",
                            now - 10,
                            &["out"],
                        ),
                        message(
                            "inactive",
                            "sibling-h",
                            "self",
                            "hidden sibling",
                            now - 5,
                            &["out"],
                        ),
                        message(
                            "to-human",
                            "root",
                            "self",
                            "hidden human",
                            now - 30,
                            &["human"],
                        ),
                        message(
                            "to-backend",
                            "root",
                            "self",
                            "hidden backend",
                            now - 35,
                            &["backend"],
                        ),
                        message(
                            "unrelated",
                            "root",
                            "chatter",
                            "hidden chatter",
                            now - 8,
                            &["out"],
                        ),
                        message(
                            "broadcast",
                            "root",
                            "chatter",
                            "hidden ambient",
                            now - 3,
                            &[],
                        ),
                    ]),
            );
            let generation = store.reserve_session_with_facts(
                &RegisterSession {
                    pubkey: "self".into(),
                    observed_harness: "pi".into(),
                    agent_slug: "pi".into(),
                    launch_channel_h: "root".into(),
                    work_root: "root".into(),
                    child_pid: None,
                    now: 1,
                },
                &crate::state::AdmittedRuntimeFacts {
                    observed_harness: "pi".into(),
                    claimed_harness: "pi".into(),
                    preset: String::new(),
                    transport: String::new(),
                    endpoint_provenance: "hook".into(),
                },
            )?;
            let lifecycle = store
                .get_session("self")?
                .context("missing session")?
                .lifecycle_epoch;
            store.set_native_resume_locator("self", "pi", "pi-session", 1)?;
            store.commit_confirmed_session_admission("self", "root", generation, lifecycle, 2)?;
            store
                .commit_confirmed_session_admission("self", "joined-h", generation, lifecycle, 2)?;
            store.set_session_title("self", "Build the focused Pi panel")?;
            Ok::<_, anyhow::Error>(())
        })
        .unwrap();

    state.with_store(|store| {
        assert_eq!(store.list_session_routes("self").unwrap().len(), 2);
        assert_eq!(
            store
                .get_session_standing("self", "joined-h")
                .unwrap()
                .unwrap()
                .state,
            crate::state::StandingState::Member,
        );
        assert_eq!(
            crate::channel_ref::full_channel_ref(store, "joined-h"),
            "#root/joined"
        );
    });

    let overview = rpc(
        &state,
        &serde_json::json!({
            "harness": "pi",
            "harness_session": "pi-session",
        }),
    )
    .await
    .unwrap();

    assert_eq!(overview["name"], "drift-pi");
    assert_eq!(overview["title"], "Build the focused Pi panel");
    assert_eq!(
        overview["channels"],
        serde_json::json!([
            { "path": "#root", "about": "Main coordination" },
            { "path": "#root/joined", "about": "Panel tests" },
        ])
    );
    assert_eq!(
        overview["interactions"],
        serde_json::json!([
            {
                "name": "bravo-codex",
                "channel": "#root/joined",
                "message": "latest to bravo",
                "age": "just now",
            },
            {
                "name": "haven-codex",
                "channel": "#root",
                "message": "latest from haven",
                "age": "just now",
            },
        ])
    );
    assert!(overview.get("relay").is_none());
    assert!(overview.get("sessions").is_none());
}
