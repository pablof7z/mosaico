use super::*;

#[test]
fn conceptual_acp_maps_to_each_harness_native_rpc_transport() {
    assert_eq!(
        mode_transport(Harness::ClaudeCode, OperationMode::Acp),
        Transport::Acp
    );
    assert_eq!(
        mode_transport(Harness::Codex, OperationMode::Acp),
        Transport::AppServer
    );
    assert_eq!(
        mode_transport(Harness::Hermes, OperationMode::Acp),
        Transport::Acp
    );
    assert_eq!(
        operation_modes(Harness::Hermes, false),
        [OperationMode::Acp, OperationMode::Pty]
    );
    assert_eq!(
        operation_modes(Harness::Kimi, false),
        [OperationMode::Acp, OperationMode::Pty]
    );
}

#[test]
fn native_profiles_only_offer_transports_that_activate_them() {
    assert_eq!(
        operation_modes(Harness::ClaudeCode, true),
        [OperationMode::Acp, OperationMode::Pty]
    );
    assert_eq!(
        operation_modes(Harness::Opencode, true),
        [OperationMode::Pty]
    );
    assert_eq!(operation_modes(Harness::Kimi, true), [OperationMode::Pty]);
}

#[test]
fn compatible_bundle_filter_never_crosses_harnesses() {
    let config: HarnessesConfig = serde_json::from_str(
        r#"{"claude-acp":{"harness":"claude-code","transport":"acp"},"codex-pty":{"harness":"codex","transport":"pty"}}"#,
    )
    .unwrap();
    assert_eq!(
        compatible_bundles(&config, Harness::ClaudeCode, Transport::Acp),
        ["claude-acp"]
    );
}

#[test]
fn editing_a_configured_agent_preserves_its_explicit_profile() {
    let row = AgentRow {
        slug: "reviewer".into(),
        agent_slug: "reviewer".into(),
        description: "Reviews".into(),
        harness: Harness::ClaudeCode,
        bundle: Some("claude-pty".into()),
        transport: Some(Transport::Pty),
        profile: Some("specialist".into()),
        per_session_key: Some(true),
        kind: AgentKind::Configured,
        native_profile: Some(crate::agent_catalog::NativeAgentProfile {
            slug: "reviewer".into(),
            use_criteria: "Reviews".into(),
            harness: Harness::ClaudeCode,
            scope: crate::agent_catalog::AgentScope::Global,
            path: "/tmp/reviewer.md".into(),
            modified_at: 1,
        }),
    };

    assert_eq!(profile_for_save(&row).as_deref(), Some("specialist"));
}
