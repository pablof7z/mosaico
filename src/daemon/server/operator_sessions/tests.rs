use super::*;
use nostr_sdk::prelude::Keys;

#[test]
fn projection_exposes_public_identity_without_private_runtime_id() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_channel("workspace", "mosaico", "", "", 1)
        .unwrap();
    store
        .upsert_channel("room", "review", "", "workspace", 2)
        .unwrap();
    store.upsert_workspace("workspace", "/repo", 3).unwrap();
    store
        .upsert_channel("skills-root", "skills", "", "", 3)
        .unwrap();
    store
        .upsert_channel("skill-dev", "skill-dev", "", "skills-root", 4)
        .unwrap();
    store.upsert_workspace("skills-root", "/skills", 4).unwrap();
    let pubkey = Keys::generate().public_key().to_hex();
    store
        .reserve_hook_session_for_test(&crate::state::RegisterSession {
            pubkey: pubkey.clone(),
            observed_harness: "codex".into(),
            agent_slug: "codex".into(),
            channel_h: "room".into(),
            child_pid: Some(42),
            transcript_path: None,
            now: 10,
        })
        .unwrap();
    store.grant_session_route(&pubkey, "skill-dev", 11).unwrap();

    let rows = project_sessions(&store, "laptop", &HashMap::new()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["pubkey"], pubkey);
    assert_eq!(rows[0]["npub"], crate::idref::npub(&pubkey).unwrap());
    assert!(rows[0].get("session_id").is_none());
    assert_eq!(rows[0]["workspaces"].as_array().unwrap().len(), 2);
    assert_eq!(rows[0]["workspaces"][0]["id"], "skills-root");
    assert_eq!(rows[0]["workspaces"][0]["path"], "/skills");
    assert_eq!(rows[0]["workspaces"][0]["channels"][0]["name"], "skill-dev");
    assert_eq!(rows[0]["workspaces"][1]["id"], "workspace");
    assert_eq!(rows[0]["workspaces"][1]["path"], "/repo");
    assert_eq!(rows[0]["workspaces"][1]["channels"][0]["name"], "review");
    assert_eq!(rows[0]["transport"], "process");
    assert!(rows[0]["endpoint"].is_null());
    assert!(rows[0]["takeover"].is_null());

    store
        .mark_runtime_stopped(
            &pubkey,
            crate::state::StopReason::OperatorKill,
            crate::util::now_secs(),
        )
        .unwrap();
    let rows = project_sessions(&store, "laptop", &HashMap::new()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["state"], "offline");
    assert_eq!(rows[0]["running"], false);
    assert_eq!(rows[0]["resumable"], false);
}

#[test]
fn stopped_session_projection_retains_project_and_restart_capability() {
    let store = Store::open_memory().unwrap();
    store.upsert_channel("root", "mosaico", "", "", 1).unwrap();
    store.upsert_workspace("root", "/repo/mosaico", 1).unwrap();
    let pubkey = Keys::generate().public_key().to_hex();
    store
        .reserve_hook_session_for_test(&crate::state::RegisterSession {
            pubkey: pubkey.clone(),
            observed_harness: "codex".into(),
            agent_slug: "codex".into(),
            channel_h: "root".into(),
            child_pid: Some(42),
            transcript_path: None,
            now: 10,
        })
        .unwrap();
    store
        .put_session_locator(
            "codex",
            crate::state::LOCATOR_NATIVE_RESUME,
            "thread-juno",
            &pubkey,
            11,
        )
        .unwrap();
    store
        .mark_runtime_stopped(&pubkey, crate::state::StopReason::HeadlessExit, 12)
        .unwrap();

    let rows = project_sessions(&store, "laptop", &HashMap::new()).unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["state"], "offline");
    assert_eq!(rows[0]["state_since"], 12);
    assert_eq!(rows[0]["resumable"], true);
    assert_eq!(rows[0]["workspaces"][0]["name"], "mosaico");
    assert_eq!(rows[0]["workspaces"][0]["path"], "/repo/mosaico");
}

#[test]
fn unhosted_resumable_projection_exposes_open_turn_takeover_state() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_channel("root", "cut-tracker", "", "", 1)
        .unwrap();
    let pubkey = Keys::generate().public_key().to_hex();
    store
        .reserve_hook_session_for_test(&crate::state::RegisterSession {
            pubkey: pubkey.clone(),
            observed_harness: "codex".into(),
            agent_slug: "codex".into(),
            channel_h: "root".into(),
            child_pid: Some(42),
            transcript_path: None,
            now: 10,
        })
        .unwrap();
    store
        .put_session_locator(
            "codex",
            crate::state::LOCATOR_NATIVE_RESUME,
            "thread-1",
            &pubkey,
            11,
        )
        .unwrap();
    let generation = store
        .get_session(&pubkey)
        .unwrap()
        .unwrap()
        .runtime_generation;
    store
        .apply_session_turn_started(&pubkey, generation, 12, None)
        .unwrap();
    let rows = project_sessions(&store, "laptop", &HashMap::new()).unwrap();

    assert_eq!(rows[0]["state"], "working");
    assert_eq!(rows[0]["state_since"], 12);
    assert_eq!(rows[0]["transport"], "process");
    assert_eq!(rows[0]["takeover"]["turn_open"], true);
    assert_eq!(rows[0]["takeover"]["turn_count"], 1);
}

#[test]
fn projection_uses_lifecycle_transition_time_instead_of_lease_times() {
    let store = Store::open_memory().unwrap();
    store.upsert_channel("root", "root", "", "", 1).unwrap();
    let pubkey = Keys::generate().public_key().to_hex();
    store
        .reserve_hook_session_for_test(&crate::state::RegisterSession {
            pubkey: pubkey.clone(),
            observed_harness: "codex".into(),
            agent_slug: "codex".into(),
            channel_h: "root".into(),
            child_pid: Some(42),
            transcript_path: None,
            now: 10,
        })
        .unwrap();
    let mut status = crate::state::Status {
        pubkey: pubkey.clone(),
        channel_h: "root".into(),
        slug: "codex".into(),
        title: "Picker status".into(),
        activity: String::new(),
        state: crate::session_state::SessionState::Suspended,
        state_since: 10,
        last_seen: 20,
        updated_at: 20,
        expiration: 200,
    };
    store.upsert_status(&status).unwrap();
    status.last_seen = 100;
    status.updated_at = 100;
    store.upsert_status(&status).unwrap();
    let rows = project_sessions(&store, "laptop", &HashMap::new()).unwrap();
    assert_eq!(rows[0]["state"], "suspended");
    assert_eq!(rows[0]["state_since"], 10);
}

#[test]
fn projection_includes_live_unbound_supervisor() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_channel("workspace", "mosaico", "", "", 1)
        .unwrap();
    store.upsert_workspace("workspace", "/repo", 1).unwrap();
    let metadata = crate::pty::LaunchMetadata {
        id: "pty-1".into(),
        socket: "/tmp/pty-1.sock".into(),
        supervisor_pid: 42,
        instance_token: String::new(),
        adopted_process_fingerprint: String::new(),
        child_pid: None,
        agent: "codex".into(),
        root: "workspace".into(),
        cwd: "/repo/subdir".into(),
        ephemeral: false,
        command: vec!["codex".into(), "--yolo".into()],
    };
    let endpoints = HashMap::from([(
        metadata.id.clone(),
        OperatorEndpoint {
            metadata,
            live: true,
        },
    )]);

    let rows = project_sessions(&store, "laptop", &endpoints).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["bound"], false);
    assert_eq!(rows[0]["handle"], "codex");
    assert_eq!(rows[0]["endpoint"]["id"], "pty-1");
    assert_eq!(rows[0]["endpoint"]["kind"], "pty");
    assert_eq!(rows[0]["workspaces"][0]["name"], "mosaico");
    assert_eq!(rows[0]["title"], "codex --yolo");
}

#[test]
fn bound_endpoint_projection_is_transport_owned_and_generic() {
    let store = Store::open_memory().unwrap();
    store.upsert_channel("root", "root", "", "", 1).unwrap();
    let pubkey = Keys::generate().public_key().to_hex();
    store
        .reserve_session_with_facts(
            &crate::state::RegisterSession {
                pubkey: pubkey.clone(),
                observed_harness: "codex".into(),
                agent_slug: "codex".into(),
                channel_h: "root".into(),
                child_pid: Some(42),
                transcript_path: None,
                now: 1,
            },
            &crate::state::AdmittedRuntimeFacts {
                observed_harness: "codex".into(),
                claimed_harness: String::new(),
                bundle: "codex-app-server".into(),
                transport: "app-server".into(),
                endpoint_provenance: "launch".into(),
            },
        )
        .unwrap();
    store
        .put_session_locator(
            "codex",
            crate::state::LOCATOR_APP_SERVER,
            "app-server-operator-test",
            &pubkey,
            2,
        )
        .unwrap();

    let rows = project_sessions(&store, "laptop", &HashMap::new()).unwrap();
    assert_eq!(rows[0]["transport"], "app-server");
    assert_eq!(rows[0]["endpoint"]["id"], "app-server-operator-test");
    assert_eq!(rows[0]["endpoint"]["kind"], "app-server");
    assert_eq!(rows[0]["endpoint"]["live"], false);
    assert_eq!(rows[0]["endpoint"]["attachable"], false);
    assert!(rows[0]["takeover"].is_null());
    assert!(rows[0].get("acp_endpoint_id").is_none());
    assert!(rows[0].get("acp_live").is_none());
}

#[test]
fn missing_hosted_locator_preserves_the_admitted_transport() {
    for transport in ["pty", "acp", "app-server"] {
        let store = Store::open_memory().unwrap();
        store.upsert_channel("root", "root", "", "", 1).unwrap();
        let pubkey = Keys::generate().public_key().to_hex();
        store
            .reserve_session_with_facts(
                &crate::state::RegisterSession {
                    pubkey: pubkey.clone(),
                    observed_harness: "codex".into(),
                    agent_slug: "codex".into(),
                    channel_h: "root".into(),
                    child_pid: Some(42),
                    transcript_path: None,
                    now: 1,
                },
                &crate::state::AdmittedRuntimeFacts {
                    observed_harness: "codex".into(),
                    claimed_harness: String::new(),
                    bundle: format!("codex-{transport}"),
                    transport: transport.into(),
                    endpoint_provenance: "launch".into(),
                },
            )
            .unwrap();

        let rows = project_sessions(&store, "laptop", &HashMap::new()).unwrap();
        assert_eq!(rows[0]["transport"], transport);
        assert!(rows[0]["endpoint"].is_null());
    }
}
