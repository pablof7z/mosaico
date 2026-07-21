use super::{ConfigRepair, Harness, InstallOpts};
use anyhow::Result;

pub(in crate::cli) fn repair_device_config() -> Result<ConfigRepair> {
    super::device_config::repair_non_interactive()
}

/// Repair one previously selected Mosaico integration without rendering. The
/// caller is responsible for deriving consent from `is_present`.
pub(in crate::cli) fn repair_integration(harness: &Harness) -> Result<()> {
    let opts = InstallOpts::default();
    match harness.id {
        "claude-code" | "codex" | "grok" => super::install_json_harness(harness, &opts, false),
        "opencode" => super::install_opencode(harness, &opts, false),
        "goose" => super::goose::install(harness, &opts, false),
        "hermes" => super::hermes::install(harness, &opts, false),
        _ => Ok(()),
    }
}
