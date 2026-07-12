use super::*;

#[test]
fn transport_kind_strings() {
    assert_eq!(TransportKind::Pty.as_str(), "pty");
    assert_eq!(TransportKind::Acp.as_str(), "acp");
}

#[test]
fn no_bundle_selects_pty() {
    // Agents without a configured harness bundle stay on the PTY transport.
    let cfg = crate::harness::config::HarnessesConfig::default();
    let t = select_transport_with(&cfg, None).unwrap();
    assert_eq!(t.kind(), TransportKind::Pty);
}

#[test]
fn builtin_slug_bundle_selects_pty() {
    // A bare harness slug (no harnesses.json entry) resolves to the PTY default.
    let cfg = crate::harness::config::HarnessesConfig::default();
    assert_eq!(
        transport_kind_for(&cfg, Some("claude")).unwrap(),
        TransportKind::Pty
    );
}

#[test]
fn acp_and_app_server_bundles_select_acp() {
    let json = r#"{
        "claude-acp": { "harness": "claude", "transport": "acp" },
        "codex-app": { "harness": "codex", "transport": "app-server" },
        "oc-pty":    { "harness": "opencode", "transport": "pty" },
        "cx-exec":   { "harness": "codex", "transport": "headless-exec" }
    }"#;
    let cfg: crate::harness::config::HarnessesConfig = serde_json::from_str(json).unwrap();
    assert_eq!(
        transport_kind_for(&cfg, Some("claude-acp")).unwrap(),
        TransportKind::Acp
    );
    assert_eq!(
        select_transport_with(&cfg, Some("codex-app"))
            .unwrap()
            .kind(),
        TransportKind::Acp
    );
    // Non-RPC transports fall to PTY.
    assert_eq!(
        transport_kind_for(&cfg, Some("oc-pty")).unwrap(),
        TransportKind::Pty
    );
    assert_eq!(
        transport_kind_for(&cfg, Some("cx-exec")).unwrap(),
        TransportKind::Pty
    );
}

#[test]
fn unknown_bundle_fails_loud() {
    let cfg = crate::harness::config::HarnessesConfig::default();
    assert!(transport_kind_for(&cfg, Some("not-a-harness")).is_err());
}

#[test]
fn pty_transport_reports_pty_kind() {
    assert_eq!(PtyTransport.kind(), TransportKind::Pty);
}

#[test]
fn acp_transport_reports_acp_kind() {
    assert_eq!(AcpTransport.kind(), TransportKind::Acp);
}

#[tokio::test]
async fn pty_resume_without_token_errors() {
    let spec = LaunchSpec {
        slug: "claude".into(),
        root: "chan".into(),
        abs_path: "/tmp".into(),
        group: None,
        ephemeral: false,
        base_command: vec!["claude".into()],
    };
    let resume = ResumeSpec {
        native_id: String::new(),
    };
    let err = PtyTransport.resume(&spec, &resume).await.unwrap_err();
    assert!(err.to_string().contains("not resumable"));
}

/// LIVE: the launch dispatch selects `AcpTransport` for an acp-bundle agent and
/// spawns a real `opencode acp` child. Gated so CI without opencode/auth skips
/// it. Run with:
///   TENEX_EDGE_RPC_LIVE=1 cargo test --lib -- --ignored \
///     session_host::transport::transport_tests::live_launch_dispatch_spawns_opencode_acp
#[tokio::test]
#[ignore]
async fn live_launch_dispatch_spawns_opencode_acp() {
    if std::env::var("TENEX_EDGE_RPC_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipping live test (set TENEX_EDGE_RPC_LIVE=1)");
        return;
    }
    // Point edge_home at a temp dir carrying a harnesses.json bundle that maps
    // opencode onto the ACP transport, so the *dispatch* (not a hand-built
    // transport) resolves to AcpTransport.
    let home = std::env::temp_dir().join(format!("te-acp-launch-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("TENEX_EDGE_HOME", &home);
    std::fs::write(
        home.join("harnesses.json"),
        r#"{ "opencode-acp": { "harness": "opencode", "transport": "acp" } }"#,
    )
    .unwrap();

    // Dispatch decision.
    let transport = select_transport(Some("opencode-acp")).unwrap();
    assert_eq!(transport.kind(), TransportKind::Acp);

    let cwd = home.join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    let spec = LaunchSpec {
        slug: "opencode-acp".into(),
        root: "live".into(),
        abs_path: cwd.to_string_lossy().into_owned(),
        group: None,
        ephemeral: true,
        base_command: vec!["opencode".into()],
    };
    let endpoint = transport.launch(&spec).await.expect("launch opencode acp");
    let ep = EndpointRef {
        kind: endpoint.kind,
        endpoint_id: endpoint.endpoint_id.clone(),
    };
    assert_eq!(endpoint.kind, TransportKind::Acp);
    assert!(
        endpoint.watch_pid.is_some(),
        "acp launch must expose the child pid as watch_pid"
    );
    assert!(
        transport.is_live(&ep),
        "freshly launched child should be live"
    );
    transport.kill(&ep).await.unwrap();
    std::env::remove_var("TENEX_EDGE_HOME");
}

#[tokio::test]
async fn acp_is_live_false_for_unknown_endpoint() {
    let ep = EndpointRef {
        kind: TransportKind::Acp,
        endpoint_id: "te-acp-nope".into(),
    };
    assert!(!AcpTransport.is_live(&ep));
    // kill of an unregistered endpoint is a no-op, not an error.
    assert!(AcpTransport.kill(&ep).await.is_ok());
}
