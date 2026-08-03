//! Home-scoped PTY supervisor reaping.
//!
//! Detached PTY supervisors intentionally survive ordinary daemon restarts so
//! live agent sessions reattach after a binary swap. Full teardown of an
//! isolated `$MOSAICO_HOME` (tests, labs, wipe) must still kill every supervisor
//! that home owns — otherwise TempDir cleanup leaves processes reparented to
//! PID 1 with a deleted binary for days.

use super::meta::{read_all_metadata, terminate_owned_supervisor};
use anyhow::Result;

/// Environment flag that makes orderly daemon shutdown reap every supervisor
/// recorded under the current `$MOSAICO_HOME`. Tests and labs set this; ordinary
/// production `daemon stop` / restart leaves it unset so sessions survive.
pub const REAP_SESSIONS_ON_STOP_ENV: &str = "MOSAICO_REAP_SESSIONS_ON_STOP";

/// Whether the current process should reap PTY supervisors on daemon stop.
pub fn reap_sessions_on_stop_enabled() -> bool {
    match std::env::var(REAP_SESSIONS_ON_STOP_ENV) {
        Ok(value) => {
            let value = value.trim();
            !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
        }
        Err(_) => false,
    }
}

/// Terminate every PTY supervisor whose metadata lives under the current
/// `$MOSAICO_HOME`. Ownership is re-checked from the live command line before
/// any signal is sent.
///
/// Ordinary `daemon stop` does **not** call this. Callers: test/lab harness
/// teardown, wipe tooling, and daemon shutdown when
/// [`REAP_SESSIONS_ON_STOP_ENV`] is set.
pub fn reap_home_supervisors() -> Result<ReapReport> {
    let mut report = ReapReport::default();
    for metadata in read_all_metadata() {
        match terminate_owned_supervisor(&metadata.id) {
            Ok(true) => report.reaped.push(metadata.id),
            Ok(false) => {}
            Err(error) => report.errors.push(format!("{}: {error:#}", metadata.id)),
        }
    }
    Ok(report)
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReapReport {
    pub reaped: Vec<String>,
    pub errors: Vec<String>,
}

impl ReapReport {
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reap_flag_defaults_off() {
        // SAFETY: unit test mutates process env under exclusive test execution.
        unsafe {
            std::env::remove_var(REAP_SESSIONS_ON_STOP_ENV);
        }
        assert!(!reap_sessions_on_stop_enabled());
    }

    #[test]
    fn reap_flag_accepts_truthy_values() {
        unsafe {
            std::env::set_var(REAP_SESSIONS_ON_STOP_ENV, "1");
        }
        assert!(reap_sessions_on_stop_enabled());
        unsafe {
            std::env::set_var(REAP_SESSIONS_ON_STOP_ENV, "0");
        }
        assert!(!reap_sessions_on_stop_enabled());
        unsafe {
            std::env::remove_var(REAP_SESSIONS_ON_STOP_ENV);
        }
    }
}
