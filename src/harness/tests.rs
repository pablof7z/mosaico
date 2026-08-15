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
    for harness in Harness::ALL {
        assert!(
            driver::all().iter().any(|driver| driver.harness == harness),
            "{} has no driver",
            harness.as_str()
        );
    }
}

#[test]
fn default_launch_has_no_implicit_args() {
    let resolved = resolve_with(
        &PresetsConfig::default(),
        Harness::Codex,
        Transport::Pty,
        None,
        None,
        scratch().path(),
    )
    .unwrap();
    assert_eq!(resolved.base_argv, ["codex"]);
    assert_eq!(resolved.preset, None);
}

#[test]
fn selected_preset_applies_only_selected_transport_args() {
    let presets: PresetsConfig = serde_json::from_str(
        r#"{"unrestricted":{"codex":{"pty":["--yolo"],"app-server":["--server-arg"]}}}"#,
    )
    .unwrap();
    let resolved = resolve_with(
        &presets,
        Harness::Codex,
        Transport::Pty,
        None,
        Some("unrestricted"),
        scratch().path(),
    )
    .unwrap();
    assert_eq!(resolved.base_argv, ["codex", "--yolo"]);
    assert_eq!(resolved.preset.as_deref(), Some("unrestricted"));
}

#[test]
fn canonical_harness_can_resolve_different_transports() {
    for transport in [Transport::Pty, Transport::AppServer] {
        let resolved = resolve_with(
            &PresetsConfig::default(),
            Harness::Codex,
            transport,
            None,
            None,
            scratch().path(),
        )
        .unwrap();
        assert_eq!(resolved.harness, Harness::Codex);
        assert_eq!(resolved.transport, transport);
    }
}

#[test]
fn pi_rpc_is_a_driver_of_the_pi_harness() {
    let pi = driver::lookup(Harness::Pi, Transport::PiRpc).unwrap();
    assert_eq!(pi.base_argv, ["pi", "--mode", "rpc"]);
    assert_eq!(pi.resume, ResumeMechanism::RpcSpawnFlag("--session"));
    assert_eq!(pi.steer, SteerPrimitive::PiRpcSteer);
    assert_eq!(pi.profile, ProfileMechanism::Unsupported);
}

#[test]
fn invalid_driver_cells_are_absent() {
    assert!(driver::lookup(Harness::Codex, Transport::Acp).is_none());
    assert!(driver::lookup(Harness::Grok, Transport::AppServer).is_none());
    assert!(driver::lookup(Harness::Pi, Transport::Acp).is_none());
    assert!(driver::lookup(Harness::Pi, Transport::AppServer).is_none());
}

#[test]
fn missing_driver_pair_fails() {
    assert!(resolve_with(
        &PresetsConfig::default(),
        Harness::Grok,
        Transport::Acp,
        None,
        None,
        scratch().path(),
    )
    .is_err());
}
