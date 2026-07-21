use super::{CheckStatus, DoctorReport};
use std::fmt::Write as _;

pub(super) fn human(report: &DoctorReport) -> String {
    let mut out = String::new();
    let verdict = if report.healthy {
        "healthy"
    } else {
        "needs attention"
    };
    writeln!(out, "mosaico doctor: {verdict}").ok();
    if let Some(home) = report
        .storage
        .get("mosaico_home")
        .and_then(|value| value.as_str())
    {
        writeln!(out, "home: {home}").ok();
    }
    if !report.repairs.is_empty() {
        writeln!(out, "repairs:").ok();
        for repair in &report.repairs {
            writeln!(out, "  - {repair}").ok();
        }
    }
    for check in &report.checks {
        let label = match check.status {
            CheckStatus::Ok => "ok",
            CheckStatus::Warning => "warn",
            CheckStatus::Error => "error",
        };
        writeln!(out, "[{label}] {}: {}", check.name, check.summary).ok();
        if let Some(path) = &check.path {
            writeln!(out, "  path: {path}").ok();
        }
        if let Some(repair) = &check.repair {
            writeln!(out, "  fix: {repair}").ok();
        }
    }
    if !report.healthy && !report.fix_attempted {
        writeln!(
            out,
            "\nRun `mosaico doctor --fix` for safe automatic repairs."
        )
        .ok();
    }
    out
}
