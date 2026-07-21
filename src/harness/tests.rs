use super::*;
use crate::session::Harness;

fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn every_declared_driver_cell_looks_up() {
    for declared in driver::all() {
        let resolved = driver::lookup(declared.harness, declared.transport).unwrap();
        assert_eq!(resolved.harness, declared.harness);
        assert_eq!(resolved.transport, declared.transport);
    }
}

#[test]
fn every_canonical_harness_has_a_driver() {
    for harness in [
        Harness::ClaudeCode,
        Harness::Codex,
        Harness::Opencode,
        Harness::Grok,
        Harness::Goose,
        Harness::Hermes,
    ] {
        assert!(
            driver::all().iter().any(|driver| driver.harness == harness),
            "{} has no driver",
            harness.as_str()
        );
    }
}

#[test]
fn invalid_driver_cells_are_absent() {
    assert!(driver::lookup(Harness::Codex, Transport::Acp).is_none());
    assert!(driver::lookup(Harness::Grok, Transport::AppServer).is_none());
    assert!(driver::lookup(Harness::Opencode, Transport::AppServer).is_none());
    assert!(driver::lookup(Harness::Goose, Transport::AppServer).is_none());
    assert!(driver::lookup(Harness::Hermes, Transport::AppServer).is_none());
}

#[test]
fn goose_uses_native_acp_with_cross_process_resume() {
    let goose = driver::lookup(Harness::Goose, Transport::Acp).unwrap();
    assert_eq!(goose.base_argv, ["goose", "acp"]);
    assert_eq!(goose.resume, ResumeMechanism::AcpSessionLoad);
    assert_eq!(goose.steer, SteerPrimitive::None);
    assert_eq!(goose.turn, TurnModel::RpcTurn);
    assert_eq!(goose.profile, ProfileMechanism::Unsupported);
}

#[test]
fn goose_interactive_driver_uses_native_session_ui() {
    let goose = driver::lookup(Harness::Goose, Transport::Pty).unwrap();
    assert_eq!(goose.base_argv, ["goose", "session"]);
    assert_eq!(
        goose.resume,
        ResumeMechanism::AppendFlags(&["--resume", "--session-id"])
    );
    assert_eq!(goose.steer, SteerPrimitive::PtyPaste);
    assert_eq!(goose.turn, TurnModel::InteractivePty);
}

#[test]
fn goose_bundle_round_trips_only_its_canonical_name() {
    let cfg: HarnessesConfig =
        serde_json::from_str(r#"{"goose-acp":{"harness":"goose","transport":"acp"}}"#).unwrap();
    let bundle = cfg.get("goose-acp").unwrap();
    assert_eq!(bundle.harness, Harness::Goose);
    assert_eq!(
        serde_json::to_value(&cfg).unwrap()["goose-acp"]["harness"],
        "goose"
    );
    assert!(resolve_with(&cfg, "goose-acp", Some("profile"), scratch().path()).is_err());
}

#[test]
fn hermes_pty_places_profile_before_bundle_args() {
    let cfg: HarnessesConfig = serde_json::from_str(
        r#"{"hermes-fast":{"harness":"hermes","transport":"pty","args":["--model","openrouter/test"]}}"#,
    )
    .unwrap();
    let resolved = resolve_with(&cfg, "hermes-fast", Some("reviewer"), scratch().path()).unwrap();
    assert_eq!(
        resolved.base_argv,
        [
            "hermes",
            "--profile",
            "reviewer",
            "--model",
            "openrouter/test"
        ]
    );
    assert_eq!(
        resolved.driver.resume,
        ResumeMechanism::AppendFlag("--resume")
    );
}

#[test]
fn hermes_acp_places_profile_before_subcommand() {
    let cfg: HarnessesConfig = serde_json::from_str(
        r#"{"hermes-rpc":{"harness":"hermes","transport":"acp","args":["--accept-hooks"]}}"#,
    )
    .unwrap();
    let resolved = resolve_with(&cfg, "hermes-rpc", Some("reviewer"), scratch().path()).unwrap();
    assert_eq!(
        resolved.base_argv,
        ["hermes", "--profile", "reviewer", "acp", "--accept-hooks"]
    );
    assert_eq!(resolved.driver.resume, ResumeMechanism::AcpSessionLoad);
}

#[test]
fn config_accepts_only_harness_transport_and_args() {
    let cfg: HarnessesConfig = serde_json::from_str(
        r#"{
          "yolo-claude": {
            "harness": "claude-code",
            "transport": "pty",
            "args": ["--dangerously-skip-permissions"]
          }
        }"#,
    )
    .unwrap();
    assert_eq!(cfg.get("yolo-claude").unwrap().harness, Harness::ClaudeCode);
    assert_eq!(cfg.get("yolo-claude").unwrap().transport, Transport::Pty);

    for removed in [
        r#"{"x":{"harness":"claude-code","transport":"pty","profile":"reviewer"}}"#,
        r#"{"x":{"harness":"codex","transport":"app-server","codex_config_profile":"planner"}}"#,
        r#"{"x":{"harness":"claude-code","transport":"pty","commands":["claude"]}}"#,
    ] {
        assert!(serde_json::from_str::<HarnessesConfig>(removed).is_err());
    }
}

#[test]
fn removed_claude_alias_is_rejected() {
    assert_eq!(Harness::from_str("claude"), Harness::Unknown);
    assert!(serde_json::from_str::<HarnessesConfig>(
        r#"{"legacy":{"harness":"claude","transport":"pty"}}"#
    )
    .is_err());
}

#[test]
fn missing_bundle_fails_without_builtin_fallback() {
    let cfg = HarnessesConfig::default();
    assert!(resolve_with(&cfg, "claude", None, scratch().path()).is_err());
}

#[test]
fn claude_pty_combines_bundle_args_and_agent_profile() {
    let cfg: HarnessesConfig = serde_json::from_str(
        r#"{"yolo-claude":{"harness":"claude-code","transport":"pty","args":["--dangerously-skip-permissions"]}}"#,
    )
    .unwrap();
    let resolved = resolve_with(&cfg, "yolo-claude", Some("reviewer"), scratch().path()).unwrap();
    assert_eq!(
        resolved.base_argv,
        [
            "claude",
            "--dangerously-skip-permissions",
            "--agent",
            "reviewer"
        ]
    );
}

#[test]
fn codex_pty_applies_profile_flag_in_code() {
    let cfg: HarnessesConfig = serde_json::from_str(
        r#"{"codex-yolo":{"harness":"codex","transport":"pty","args":["--yolo"]}}"#,
    )
    .unwrap();
    let resolved = resolve_with(&cfg, "codex-yolo", Some("reviewer"), scratch().path()).unwrap();
    assert_eq!(
        resolved.base_argv,
        ["codex", "--yolo", "--profile", "reviewer"]
    );
}

#[test]
fn missing_agent_profile_uses_harness_default() {
    let cfg: HarnessesConfig =
        serde_json::from_str(r#"{"claude":{"harness":"claude-code","transport":"pty"}}"#).unwrap();
    let resolved = resolve_with(&cfg, "claude", None, scratch().path()).unwrap();
    assert_eq!(resolved.base_argv, ["claude"]);
}

#[test]
fn codex_app_server_stages_named_profile() {
    let source = scratch();
    let target = scratch();
    std::fs::write(source.path().join("config.toml"), "model = 'base'\n").unwrap();
    std::fs::write(
        source.path().join("planner.config.toml"),
        "model = 'planner'\nsandbox_mode = 'read-only'\n",
    )
    .unwrap();
    let cfg: HarnessesConfig =
        serde_json::from_str(r#"{"codex-rpc":{"harness":"codex","transport":"app-server"}}"#)
            .unwrap();
    let resolved = resolve_with_codex_home(
        &cfg,
        "codex-rpc",
        Some("planner"),
        target.path(),
        Some(source.path()),
    )
    .unwrap();
    assert_eq!(resolved.base_argv, ["codex", "app-server"]);
    resolved.profile.materialize().unwrap();
    let staged = std::fs::read_to_string(target.path().join("codex-home/config.toml")).unwrap();
    assert!(staged.contains("model = \"planner\""));
    assert!(staged.contains("sandbox_mode = \"read-only\""));
}

#[test]
fn unsupported_profile_pair_fails_loud() {
    let cfg: HarnessesConfig =
        serde_json::from_str(r#"{"claude-rpc":{"harness":"claude-code","transport":"acp"}}"#)
            .unwrap();
    assert!(resolve_with(&cfg, "claude-rpc", Some("reviewer"), scratch().path()).is_err());
    assert!(resolve_with(&cfg, "claude-rpc", None, scratch().path()).is_ok());
}

#[test]
fn native_selector_uses_only_supported_driver_cells() {
    let cfg: HarnessesConfig =
        serde_json::from_str(r#"{"claude":{"harness":"claude-code","transport":"pty"}}"#).unwrap();
    let scratch = scratch();
    let mut resolved = resolve_with(&cfg, "claude", None, scratch.path()).unwrap();
    apply_native_agent(
        &mut resolved,
        &crate::agent_catalog::NativeAgentActivation::NativeSelector {
            name: "reviewer".into(),
        },
        scratch.path(),
    )
    .unwrap();
    assert_eq!(resolved.base_argv, ["claude", "--agent", "reviewer"]);
}

#[test]
fn claude_acp_defers_native_agent_to_session_new() {
    let cfg: HarnessesConfig =
        serde_json::from_str(r#"{"claude-acp":{"harness":"claude-code","transport":"acp"}}"#)
            .unwrap();
    let scratch = scratch();
    let mut resolved = resolve_with(&cfg, "claude-acp", None, scratch.path()).unwrap();
    apply_native_agent(
        &mut resolved,
        &crate::agent_catalog::NativeAgentActivation::NativeSelector {
            name: "reviewer".into(),
        },
        scratch.path(),
    )
    .unwrap();
    assert_eq!(
        resolved.base_argv,
        ["npx", "--yes", "@agentclientprotocol/claude-agent-acp"]
    );
    assert!(supports_native_agent(Harness::ClaudeCode, Transport::Acp));
    assert!(!supports_native_agent(Harness::Opencode, Transport::Acp));
}

#[test]
fn codex_app_server_defers_custom_agent_to_thread_start() {
    let cfg: HarnessesConfig =
        serde_json::from_str(r#"{"codex-rpc":{"harness":"codex","transport":"app-server"}}"#)
            .unwrap();
    let scratch = scratch();
    let mut resolved = resolve_with(&cfg, "codex-rpc", None, scratch.path()).unwrap();
    apply_native_agent(
        &mut resolved,
        &crate::agent_catalog::NativeAgentActivation::CodexRoot(
            crate::agent_catalog::CodexRootConfig {
                developer_instructions: "Review carefully".into(),
                config: toml::from_str("model = 'gpt-5.4'").unwrap(),
            },
        ),
        scratch.path(),
    )
    .unwrap();

    assert_eq!(resolved.base_argv, ["codex", "app-server"]);
    assert!(resolved.profile.files.is_empty());
    assert!(resolved.profile.codex_home.is_none());
}
