use super::*;

#[test]
fn supports_native_pty_and_acp_with_current_resume_contracts() {
    let pty = driver::lookup(Harness::Kimi, Transport::Pty).unwrap();
    assert_eq!(pty.base_argv, ["kimi"]);
    assert_eq!(pty.resume, ResumeMechanism::AppendFlag("--session"));
    assert_eq!(pty.steer, SteerPrimitive::PtyPaste);
    assert_eq!(pty.profile, ProfileMechanism::CliFlag { flag: "--agent" });

    let acp = driver::lookup(Harness::Kimi, Transport::Acp).unwrap();
    assert_eq!(acp.base_argv, ["kimi", "acp"]);
    assert_eq!(acp.resume, ResumeMechanism::AcpSessionLoad);
    assert_eq!(acp.turn, TurnModel::RpcTurn);
    assert_eq!(acp.profile, ProfileMechanism::Unsupported);
}

#[test]
fn native_profile_uses_pty_agent_selector_and_acp_rejects_it() {
    let pty: HarnessesConfig =
        serde_json::from_str(r#"{"kimi-pty":{"harness":"kimi","transport":"pty"}}"#).unwrap();
    let resolved = resolve_with(&pty, "kimi-pty", Some("reviewer"), scratch().path()).unwrap();
    assert_eq!(resolved.base_argv, ["kimi", "--agent", "reviewer"]);

    let acp: HarnessesConfig =
        serde_json::from_str(r#"{"kimi-acp":{"harness":"kimi","transport":"acp"}}"#).unwrap();
    assert!(resolve_with(&acp, "kimi-acp", Some("reviewer"), scratch().path()).is_err());
}
