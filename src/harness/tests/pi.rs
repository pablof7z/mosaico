use super::*;

#[test]
fn exposes_native_pty_and_rpc_without_named_profiles() {
    let cfg: HarnessesConfig = serde_json::from_str(
        r#"{
          "pi-pty":{"harness":"pi","transport":"pty"},
          "pi-rpc":{"harness":"pi","transport":"pi-rpc"}
        }"#,
    )
    .unwrap();
    let scratch = scratch();

    let pty = resolve_with(&cfg, "pi-pty", None, scratch.path()).unwrap();
    assert_eq!(pty.base_argv, ["pi"]);
    assert_eq!(pty.driver.resume, ResumeMechanism::AppendFlag("--session"));
    assert_eq!(pty.driver.steer, SteerPrimitive::PtyPaste);
    assert_eq!(pty.driver.profile, ProfileMechanism::Unsupported);

    let rpc = resolve_with(&cfg, "pi-rpc", None, scratch.path()).unwrap();
    assert_eq!(rpc.base_argv, ["pi", "--mode", "rpc"]);
    assert_eq!(
        rpc.driver.resume,
        ResumeMechanism::RpcSpawnFlag("--session")
    );
    assert_eq!(rpc.driver.steer, SteerPrimitive::PiRpcSteer);
    assert_eq!(rpc.driver.turn, TurnModel::RpcTurn);
    assert_eq!(rpc.driver.profile, ProfileMechanism::Unsupported);

    assert!(resolve_with(&cfg, "pi-pty", Some("reviewer"), scratch.path()).is_err());
    assert!(resolve_with(&cfg, "pi-rpc", Some("reviewer"), scratch.path()).is_err());
}
