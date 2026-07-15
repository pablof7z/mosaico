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
fn invalid_driver_cells_are_absent() {
    assert!(driver::lookup(Harness::Codex, Transport::Acp).is_none());
    assert!(driver::lookup(Harness::Grok, Transport::AppServer).is_none());
    assert!(driver::lookup(Harness::Opencode, Transport::AppServer).is_none());
}

#[test]
fn config_accepts_only_harness_transport_and_args() {
    let cfg: HarnessesConfig = serde_json::from_str(
        r#"{
          "yolo-claude": {
            "harness": "claude",
            "transport": "pty",
            "args": ["--dangerously-skip-permissions"]
          }
        }"#,
    )
    .unwrap();
    assert_eq!(cfg.get("yolo-claude").unwrap().harness, Harness::ClaudeCode);
    assert_eq!(cfg.get("yolo-claude").unwrap().transport, Transport::Pty);

    for removed in [
        r#"{"x":{"harness":"claude","transport":"pty","profile":"reviewer"}}"#,
        r#"{"x":{"harness":"codex","transport":"app-server","codex_config_profile":"planner"}}"#,
        r#"{"x":{"harness":"claude","transport":"pty","commands":["claude"]}}"#,
    ] {
        assert!(serde_json::from_str::<HarnessesConfig>(removed).is_err());
    }
}

#[test]
fn missing_bundle_fails_without_builtin_fallback() {
    let cfg = HarnessesConfig::default();
    assert!(resolve_with(&cfg, "claude", None, scratch().path()).is_err());
}

#[test]
fn claude_pty_combines_bundle_args_and_agent_profile() {
    let cfg: HarnessesConfig = serde_json::from_str(
        r#"{"yolo-claude":{"harness":"claude","transport":"pty","args":["--dangerously-skip-permissions"]}}"#,
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
        serde_json::from_str(r#"{"claude":{"harness":"claude","transport":"pty"}}"#).unwrap();
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
        serde_json::from_str(r#"{"claude-rpc":{"harness":"claude","transport":"acp"}}"#).unwrap();
    assert!(resolve_with(&cfg, "claude-rpc", Some("reviewer"), scratch().path()).is_err());
    assert!(resolve_with(&cfg, "claude-rpc", None, scratch().path()).is_ok());
}
