//! `probe` RPC arm — the hidden diagnostic surface over the reconciler frontier
//! (frontier design §4). The CLI (`tenex-edge probe <verb>`) forwards a `verb`
//! plus verb params here; this module dispatches to one implementation file per
//! verb, each kept small and independently testable:
//!
//! * `stats`    — aggregate value evidence over the all-commit ledger (§4.1).
//! * `oracle`   — the incremental-equals-full correctness check, live (§4.6).
//! * `seams`    — authority-frontier registrations + host-seam coverage (§4.5).
//! * `replay`   — replay a stored input capsule and optionally export a trace (§4.4).
//! * `simulate` — dry-run a fact via `tx.preview()`; the keystone (§3).
//! * `why`      — live causality for a surface handle (§4.3).
//! * `state`    — live values per surface: owners/refcounts, status inputs (§4.3).

use super::DaemonState;
use anyhow::{Context, Result};
use serde_json::Value;
use std::sync::Arc;

mod acid;
mod artifact;
mod cursor_artifact;
mod diff;
mod oracle;
mod outbox_acid;
mod outbox_artifact;
mod replay;
mod seams;
mod simulate;
mod state;
mod stats;
mod why;

pub(in crate::daemon::server) use oracle::oracle_report;

/// The reconciler surfaces the ledger records; `stats` with no `--surface`
/// reports all of them.
pub(super) const SURFACES: [&str; 6] = [
    "status",
    "subscriptions",
    "hook_context",
    "turn_lifecycle",
    "cursor",
    "outbox",
];

/// Route a `probe` RPC to its verb. `params` carries `{"verb": <str>, ...}`.
pub(in crate::daemon::server) fn rpc_probe(
    state: &Arc<DaemonState>,
    params: &Value,
) -> Result<Value> {
    let verb = params
        .get("verb")
        .and_then(Value::as_str)
        .context("probe: missing `verb` param")?;
    match verb {
        "stats" => {
            let surface = params.get("surface").and_then(Value::as_str);
            let since = params.get("since").and_then(Value::as_i64).unwrap_or(0);
            state.with_store(|s| stats::stats_value(s, surface, since))
        }
        "oracle" => Ok(oracle::oracle_value(state)),
        "seams" => Ok(seams::seams_value()),
        "diff" => diff::diff_value(state, params),
        "acid" => acid::acid_value(state, params),
        "replay" => replay::replay_value(state, params),
        "simulate" => simulate::simulate_value(state, params),
        "why" => why::why_value(state, params),
        "state" => state::state_value(state, params),
        other => Err(anyhow::anyhow!("probe: unknown verb `{other}`")),
    }
}

pub(in crate::daemon::server) fn doctor_summary(
    state: &Arc<DaemonState>,
) -> Result<serde_json::Value> {
    let since = today_start_millis();
    state.with_store(|s| stats::doctor_summary_value(s, since))
}

fn today_start_millis() -> i64 {
    const DAY_MS: u64 = 86_400_000;
    let now = crate::util::now_millis();
    ((now / DAY_MS) * DAY_MS) as i64
}

/// Shared param helper: a required string field.
fn required_str<'a>(params: &'a Value, key: &str) -> Result<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .with_context(|| format!("probe: missing `{key}` param"))
}

#[cfg(test)]
mod tests;
