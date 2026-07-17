use super::*;

#[path = "tests/live.rs"]
mod live;

#[test]
fn transport_kind_strings() {
    assert_eq!(TransportKind::Pty.as_str(), "pty");
    assert_eq!(TransportKind::Acp.as_str(), "acp");
    assert_eq!(TransportKind::AppServer.as_str(), "app-server");
    assert_eq!(TransportKind::Pty.locator_kind(), crate::state::LOCATOR_PTY);
    assert_eq!(TransportKind::Acp.locator_kind(), crate::state::LOCATOR_ACP);
    assert_eq!(
        TransportKind::AppServer.locator_kind(),
        crate::state::LOCATOR_APP_SERVER
    );
    assert_eq!(
        TransportKind::from_locator_kind(crate::state::LOCATOR_ACP),
        Some(TransportKind::Acp)
    );
    assert_eq!(serde_json::to_value(TransportKind::Acp).unwrap(), "acp");
    assert_eq!(
        serde_json::to_value(TransportKind::AppServer).unwrap(),
        "app-server"
    );
    assert_eq!(
        serde_json::from_value::<TransportKind>(serde_json::json!("pty")).unwrap(),
        TransportKind::Pty
    );
}

#[test]
fn persisted_locator_selects_the_transport_without_agent_config() {
    for kind in TransportKind::ALL {
        let locator = crate::state::SessionLocator {
            harness: "codex".into(),
            locator_kind: kind.locator_kind().into(),
            locator_value: format!("{}-owned-endpoint", kind.as_str()),
            pubkey: "pk".into(),
            created_at: 1,
        };

        let (transport, endpoint) = transport_for_locator(&locator).expect("hosted locator");
        assert_eq!(transport.kind(), kind);
        assert_eq!(endpoint.kind, kind);
        assert_eq!(endpoint.endpoint_id, locator.locator_value);
    }
}

#[test]
fn admitted_hosted_transport_remains_distinct_when_its_locator_is_missing() {
    for kind in TransportKind::ALL {
        let store = crate::state::Store::open_memory().unwrap();
        let pubkey = format!("pk-missing-{}", kind.as_str());
        store
            .reserve_session_with_facts(
                &crate::state::RegisterSession {
                    pubkey: pubkey.clone(),
                    observed_harness: "codex".into(),
                    agent_slug: "codex".into(),
                    channel_h: "root".into(),
                    child_pid: Some(std::process::id() as i32),
                    transcript_path: None,
                    now: 1,
                },
                &crate::state::AdmittedRuntimeFacts {
                    observed_harness: "codex".into(),
                    claimed_harness: String::new(),
                    bundle: format!("codex-{}", kind.as_str()),
                    transport: kind.as_str().into(),
                    endpoint_provenance: "launch".into(),
                },
            )
            .unwrap();
        let session = store.get_session(&pubkey).unwrap().unwrap();

        match hosted_endpoint_for(&store, &session).unwrap() {
            HostedEndpoint::Unavailable { kind: actual } => assert_eq!(actual, kind),
            HostedEndpoint::Unhosted | HostedEndpoint::Resolved { .. } => {
                panic!("missing {} locator lost admitted transport", kind.as_str())
            }
        }
    }
}

#[test]
fn hosted_locator_lookup_errors_are_not_collapsed_to_unhosted() {
    let scratch = tempfile::tempdir().unwrap();
    let database = scratch.path().join("state.db");
    let store = crate::state::Store::open(&database).unwrap();
    store
        .reserve_session_with_facts(
            &crate::state::RegisterSession {
                pubkey: "pk-broken-locator-table".into(),
                observed_harness: "codex".into(),
                agent_slug: "codex".into(),
                channel_h: "root".into(),
                child_pid: Some(std::process::id() as i32),
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
    let session = store
        .get_session("pk-broken-locator-table")
        .unwrap()
        .unwrap();

    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch("DROP TABLE session_locators")
        .unwrap();

    let error = match hosted_endpoint_for(&store, &session) {
        Err(error) => error,
        Ok(_) => panic!("broken locator lookup was collapsed to an endpoint state"),
    };
    assert!(error.to_string().contains("session_locators"), "{error:#}");
}

#[test]
fn missing_bundle_fails_without_pty_fallback() {
    let cfg = crate::harness::HarnessesConfig::default();
    assert!(select_transport_with(&cfg, "claude").is_err());
}

#[test]
fn configured_bundles_select_exact_transport() {
    let cfg: crate::harness::HarnessesConfig = serde_json::from_str(
        r#"{
          "claude-pty":{"harness":"claude","transport":"pty"},
          "claude-acp":{"harness":"claude","transport":"acp"},
          "codex-app":{"harness":"codex","transport":"app-server"}
        }"#,
    )
    .unwrap();
    assert_eq!(
        transport_kind_for(&cfg, "claude-pty").unwrap(),
        TransportKind::Pty
    );
    assert_eq!(
        transport_kind_for(&cfg, "claude-acp").unwrap(),
        TransportKind::Acp
    );
    assert_eq!(
        select_transport_with(&cfg, "codex-app").unwrap().kind(),
        TransportKind::AppServer
    );
}

#[test]
fn rpc_spawn_uses_the_admitted_plan_after_config_mutation() {
    let mut cfg: crate::harness::HarnessesConfig = serde_json::from_str(
        r#"{"codex-rpc":{"harness":"codex","transport":"app-server","args":["--admitted"]}}"#,
    )
    .unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let mut resolved =
        crate::harness::resolve_with(&cfg, "codex-rpc", None, scratch.path()).unwrap();
    let prepared = RpcTransport::new(TransportKind::AppServer)
        .prepare_launch(&mut resolved, "endpoint".into())
        .unwrap();

    cfg.bundles.get_mut("codex-rpc").unwrap().args = vec!["--mutated".into()];
    let spec = LaunchSpec {
        slug: "reviewer".into(),
        native_agent: None,
        root: "chan".into(),
        abs_path: "/tmp".into(),
        group: None,
        ephemeral: false,
        session_name: None,
        base_command: vec!["codex".into(), "app-server".into()],
        pubkey: "33".repeat(32),
        agent_nsec: "test-agent-nsec".into(),
        prepared,
    };
    let callbacks = crate::rpc_harness::Callbacks::allow_all(scratch.path().to_path_buf());
    let spawn = super::acp::RpcTransport::spawn_config(&spec, callbacks).unwrap();

    assert_eq!(spawn.program, "codex");
    assert_eq!(spawn.args, ["app-server", "--admitted"]);
    assert!(!spawn.args.iter().any(|arg| arg == "--mutated"));
}

#[tokio::test]
async fn rpc_unknown_endpoints_are_not_live() {
    for kind in [TransportKind::Acp, TransportKind::AppServer] {
        let transport = transport_for_kind(kind);
        let ep = EndpointRef {
            kind,
            endpoint_id: format!("{}-nope", kind.as_str()),
        };
        assert!(!transport.is_live(&ep));
        assert!(transport.kill(&ep).await.is_ok());
    }
}

#[tokio::test]
async fn transport_matrix_routes_liveness_delivery_and_kill_through_the_trait() {
    for kind in TransportKind::ALL {
        let transport = transport_for_kind(kind);
        let endpoint = EndpointRef {
            kind,
            endpoint_id: format!("missing-{}-endpoint", kind.as_str()),
        };

        assert_eq!(transport.kind(), kind);
        assert!(!transport.is_live(&endpoint));
        assert!(transport
            .deliver(&endpoint, "deterministic matrix probe", true)
            .await
            .is_err());
        assert!(transport.kill(&endpoint).await.is_ok());
    }
}

#[tokio::test]
async fn pty_transport_reports_a_controlled_socket_live_and_delivers_to_it() {
    let scratch = tempfile::tempdir().unwrap();
    let socket = scratch.path().join("pty-fixture.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let (delivered_tx, delivered_rx) = std::sync::mpsc::channel();
    let fixture = std::thread::spawn(move || {
        use std::io::Read as _;

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    let mut frame = Vec::new();
                    stream.read_to_end(&mut frame).unwrap();
                    if frame.starts_with(b"INJECT ") {
                        delivered_tx.send(frame).unwrap();
                        return;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(error) => panic!("PTY fixture accept failed: {error}"),
            }
        }
        panic!("PTY fixture did not receive delivery before deadline");
    });

    let endpoint = EndpointRef {
        kind: TransportKind::Pty,
        endpoint_id: socket.to_string_lossy().into_owned(),
    };
    assert!(PtyTransport.is_live(&endpoint));
    PtyTransport
        .deliver(&endpoint, "positive PTY delivery", false)
        .await
        .unwrap();

    let delivered = delivered_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("PTY fixture delivery");
    assert!(delivered.starts_with(b"INJECT "));
    assert!(delivered
        .windows(b"positive PTY delivery".len())
        .any(|window| window == b"positive PTY delivery"));
    fixture.join().unwrap();
}
