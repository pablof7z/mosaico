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
          "codex-app":{"harness":"codex","transport":"app-server"},
          "codex-exec":{"harness":"codex","transport":"headless-exec"}
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
    assert!(select_transport_with(&cfg, "codex-exec").is_err());
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
        base_command: vec!["codex".into(), "app-server".into()],
        pubkey: "33".repeat(32),
        agent_nsec: "test-agent-nsec".into(),
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
