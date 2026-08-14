use anyhow::Result;

pub(crate) const LOCATOR_NATIVE_RESUME: &str = "native_resume";
pub(crate) const LOCATOR_PTY: &str = "pty";
pub(crate) const LOCATOR_ACP: &str = "acp";
pub(crate) const LOCATOR_APP_SERVER: &str = "app_server";
pub(crate) const LOCATOR_PI_RPC: &str = "pi_rpc";
pub(crate) const LOCATOR_PID: &str = "pid";

pub(super) fn validate_locator_kind(locator_kind: &str) -> Result<()> {
    match locator_kind {
        LOCATOR_NATIVE_RESUME
        | LOCATOR_PTY
        | LOCATOR_ACP
        | LOCATOR_APP_SERVER
        | LOCATOR_PI_RPC
        | LOCATOR_PID => Ok(()),
        _ => anyhow::bail!("unknown session locator kind {locator_kind:?}"),
    }
}
