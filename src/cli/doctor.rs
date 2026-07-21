//! Agent-usable health diagnosis and safe repair for a Mosaico installation.

mod config;
mod render;
mod repair;

use anyhow::{bail, Result};
use clap::Args;
use serde::Serialize;

#[derive(Args, Debug)]
pub(in crate::cli) struct DoctorArgs {
    /// Repair Mosaico-owned config, skill, hooks, plugins, and daemon wiring.
    #[arg(long)]
    fix: bool,
    /// Print one machine-readable report to stdout.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum CheckStatus {
    Ok,
    Warning,
    Error,
}

#[derive(Debug, Serialize)]
struct Check {
    name: String,
    status: CheckStatus,
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    repair: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
}

impl Check {
    fn new(name: impl Into<String>, status: CheckStatus, summary: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status,
            summary: summary.into(),
            repair: None,
            path: None,
            state: None,
        }
    }

    fn repair(mut self, repair: impl Into<String>) -> Self {
        self.repair = Some(repair.into());
        self
    }

    fn target(mut self, path: impl Into<String>, state: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self.state = Some(state.into());
        self
    }
}

#[derive(Debug, Serialize)]
struct DoctorReport {
    healthy: bool,
    fix_attempted: bool,
    storage: serde_json::Value,
    repairs: Vec<String>,
    checks: Vec<Check>,
}

pub(in crate::cli) async fn doctor(args: DoctorArgs) -> Result<()> {
    let repair = if args.fix {
        repair::run().await
    } else {
        repair::Attempt::default()
    };

    let mut report = diagnose(args.fix, repair.actions).await;
    if let Some(error) = repair.error {
        report.checks.push(
            Check::new(
                "repair",
                CheckStatus::Error,
                format!("repair failed: {error:#}"),
            )
            .repair(
                "resolve the reported file or command error, then run `mosaico doctor --fix` again",
            ),
        );
        report.healthy = false;
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render::human(&report));
    }

    if report.healthy {
        Ok(())
    } else {
        bail!("doctor found unhealthy checks")
    }
}

async fn diagnose(fix_attempted: bool, repairs: Vec<String>) -> DoctorReport {
    let storage = serde_json::to_value(crate::daemon::storage_paths::StoragePaths::current())
        .unwrap_or(serde_json::Value::Null);
    let mut checks = Vec::new();
    let config_ready = config::inspect(&crate::config::config_path(), &mut checks);
    inspect_installation(&mut checks);
    if config_ready {
        inspect_daemon(&mut checks).await;
    } else {
        checks.push(
            Check::new(
                "daemon",
                CheckStatus::Error,
                "not started because the device configuration is unusable",
            )
            .repair("run `mosaico doctor --fix`, then re-run `mosaico doctor`"),
        );
    }
    DoctorReport {
        healthy: !checks
            .iter()
            .any(|check| check.status == CheckStatus::Error),
        fix_attempted,
        storage,
        repairs,
        checks,
    }
}

fn inspect_installation(checks: &mut Vec<Check>) {
    match super::install::skill_health() {
        Ok(health) => {
            for target in health.targets {
                let (status, state) = match target.state {
                    super::install::SkillHealthState::Healthy => (CheckStatus::Ok, "healthy"),
                    super::install::SkillHealthState::Missing => (CheckStatus::Error, "missing"),
                    super::install::SkillHealthState::Stale => (CheckStatus::Error, "stale"),
                };
                let mut check = Check::new(
                    format!("skill.{}", target.label),
                    status,
                    format!("runtime skill target is {state}"),
                )
                .target(target.path.display().to_string(), state);
                if status == CheckStatus::Error {
                    check = check
                        .repair("run `mosaico doctor --fix` to restore the bundled runtime skill");
                }
                checks.push(check);
            }
        }
        Err(error) => checks.push(Check::new(
            "skill",
            CheckStatus::Error,
            format!("cannot inspect skill installation: {error:#}"),
        )),
    }

    match super::install::harnesses() {
        Ok(harnesses) => {
            let mut detected = 0usize;
            for harness in harnesses.into_iter().filter(|harness| harness.detected) {
                detected += 1;
                let installed = super::install::is_installed(&harness);
                let present = super::install::is_present(&harness);
                let status = if installed {
                    CheckStatus::Ok
                } else if present {
                    CheckStatus::Error
                } else {
                    CheckStatus::Warning
                };
                let summary = if installed {
                    "integration installed"
                } else if present {
                    "selected integration is stale or incomplete"
                } else {
                    "detected but not selected for Mosaico hooks"
                };
                let state = if installed {
                    "healthy"
                } else if present {
                    "stale"
                } else {
                    "not-selected"
                };
                let mut check = Check::new(format!("harness.{}", harness.id), status, summary)
                    .target(harness.config_path.display().to_string(), state);
                if present && !installed {
                    check = check.repair(
                        "run `mosaico doctor --fix` to rewrite Mosaico-owned integration entries",
                    );
                } else if !present {
                    check = check.repair(format!(
                        "run `mosaico setup --harness {}` only if the user opts into this harness",
                        harness.id
                    ));
                }
                checks.push(check);
            }
            if detected == 0 {
                checks.push(Check::new(
                    "harness",
                    CheckStatus::Warning,
                    "no hook-based harness was detected; current Goose and remote MCP setup uses non-hook transports",
                ));
            }
        }
        Err(error) => checks.push(Check::new(
            "harness",
            CheckStatus::Error,
            format!("cannot inspect harnesses: {error:#}"),
        )),
    }
}

async fn inspect_daemon(checks: &mut Vec<Check>) {
    let mut client = match crate::daemon::client::Client::connect_or_spawn().await {
        Ok(client) => client,
        Err(error) => {
            checks.push(
                Check::new(
                    "daemon",
                    CheckStatus::Error,
                    format!("cannot connect or start: {error:#}"),
                )
                .repair("run `mosaico doctor --fix` for a session-preserving daemon restart"),
            );
            return;
        }
    };
    checks.push(Check::new(
        "daemon",
        CheckStatus::Ok,
        "daemon is running and compatible",
    ));
    match client.call("doctor", serde_json::json!({})).await {
        Ok(probe) => {
            let publish = probe["publish"].as_str().unwrap_or("missing result");
            checks.push(relay_check(
                "relay.publish",
                publish,
                publish.starts_with("OK ("),
                "verify relay reachability, authorization, and the relays in config.json",
            ));
            let readback = probe["readback"].as_str().unwrap_or("missing result");
            checks.push(relay_check(
                "relay.readback",
                readback,
                readback_healthy(readback),
                "verify relay read access and retry after connectivity is restored",
            ));
        }
        Err(error) => checks.push(Check::new(
            "relay.probe",
            CheckStatus::Error,
            format!("daemon probe failed: {error:#}"),
        )),
    }
}

fn relay_check(name: &str, summary: &str, healthy: bool, repair: &str) -> Check {
    let status = if healthy {
        CheckStatus::Ok
    } else if summary.starts_with("SKIP ") {
        CheckStatus::Warning
    } else {
        CheckStatus::Error
    };
    let check = Check::new(name, status, summary);
    if status == CheckStatus::Error {
        check.repair(repair)
    } else {
        check
    }
}

fn readback_healthy(value: &str) -> bool {
    value
        .split_whitespace()
        .next()
        .and_then(|count| count.parse::<usize>().ok())
        .is_some_and(|count| count > 0)
}

#[cfg(test)]
#[path = "doctor/tests.rs"]
mod tests;
