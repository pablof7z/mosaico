use super::*;
use crate::harness::driver::{ProfileMechanism, ResumeMechanism};
use crate::test_env::EnvGuard;

#[test]
fn named_hermes_smoke_applies_profile_to_every_acp_process() {
    let home = tempfile::tempdir().unwrap();
    let mut env = EnvGuard::set("MOSAICO_HOME", home.path());
    env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");

    let resolved = resolve_rpc(
        "hermes",
        Some("builder"),
        None,
        &home.path().join("scratch"),
    )
    .expect("resolve named Hermes ACP smoke");

    assert_eq!(
        resolved.base_argv,
        ["hermes", "--profile", "builder", "acp"]
    );
    assert_eq!(resolved.driver.resume, ResumeMechanism::AcpSessionLoad);
    assert_eq!(
        resolved.driver.profile,
        ProfileMechanism::CliGlobalFlag { flag: "--profile" }
    );
}
