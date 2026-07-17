use super::*;

#[path = "tests/live.rs"]
mod live;

#[test]
fn transport_kind_strings() {
    assert_eq!(TransportKind::Pty.as_str(), "pty");
    assert_eq!(TransportKind::Acp.as_str(), "acp");
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
        TransportKind::Acp
    );
}

#[test]
fn acp_resolves_driver_from_bundle_not_agent_slug() {
    let cfg: crate::harness::HarnessesConfig =
        serde_json::from_str(r#"{"codex-rpc":{"harness":"codex","transport":"app-server"}}"#)
            .unwrap();
    let spec = LaunchSpec {
        slug: "reviewer".into(),
        bundle: "codex-rpc".into(),
        profile: Some("planner".into()),
        native_agent: None,
        root: "chan".into(),
        abs_path: "/tmp".into(),
        group: None,
        ephemeral: false,
        session_name: None,
        base_command: vec!["codex".into(), "app-server".into()],
        pubkey: "33".repeat(32),
        agent_nsec: "test-agent-nsec".into(),
        pty: PtyLaunchSpec::default(),
    };
    assert_eq!(super::acp::bundle_name(&spec), "codex-rpc");
    let scratch = tempfile::tempdir().unwrap();
    let resolved =
        crate::harness::resolve_with(&cfg, super::acp::bundle_name(&spec), None, scratch.path())
            .unwrap();
    assert_eq!(resolved.harness, crate::session::Harness::Codex);
    assert!(crate::harness::resolve_with(&cfg, &spec.slug, None, scratch.path()).is_err());
}

#[tokio::test]
async fn acp_unknown_endpoint_is_not_live() {
    let ep = EndpointRef {
        kind: TransportKind::Acp,
        endpoint_id: "acp-nope".into(),
    };
    assert!(!AcpTransport.is_live(&ep));
    assert!(AcpTransport.kill(&ep).await.is_ok());
}

#[tokio::test]
async fn transport_matrix_routes_liveness_delivery_and_kill_through_the_trait() {
    for kind in [TransportKind::Pty, TransportKind::Acp] {
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
