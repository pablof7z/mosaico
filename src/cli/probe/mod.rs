//! `tenex-edge probe` — hidden diagnostic surface over the reconciler frontier
//! (frontier design §4). A thin client: it forwards a verb + params to the
//! daemon's `probe` RPC and renders the JSON it returns. `--json` (global) emits
//! the raw daemon JSON instead of the human view.
//!
//! Verbs: `stats` (aggregate value evidence, §4.1), `oracle` (live
//! incremental-equals-full correctness, §4.6), `seams` (frontier modes, §4.5),
//! `simulate` (dry-run a fact via `tx.preview()`, the keystone, §3), `diff` /
//! `acid` (counterfactual checks), `why` (live causality for a handle, §4.3),
//! and `state` (live per-surface values, §4.3).

mod render;
mod state_render;
mod stats_render;

use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Args)]
pub(in crate::cli) struct ProbeArgs {
    #[command(subcommand)]
    action: ProbeAction,
    /// Emit the raw JSON the daemon returned instead of the human view.
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand)]
enum ProbeAction {
    /// Aggregate value evidence per surface over the all-commit ledger:
    /// commits, effectful vs suppressed no-ops, command/output totals, latency.
    Stats {
        /// One surface (`status` | `subscriptions` | `session_start` | ...); omit for all.
        #[arg(long)]
        surface: Option<String>,
        /// Only count commits with `created_at` at/after this unix-millis stamp.
        #[arg(long, default_value = "0")]
        since: i64,
    },
    /// Run the incremental-equals-full oracle on each live reconciler graph.
    Oracle {
        /// Advisory/no-op: accepted for symmetry but IGNORED — the oracle always
        /// runs live over the daemon-held graphs regardless of this flag.
        #[arg(long)]
        now: bool,
        /// Restrict reporting to one surface (advisory; all are checked).
        #[arg(long)]
        surface: Option<String>,
    },
    /// Show the code-owned authority-frontier registrations and bypass risks.
    Seams,
    /// Replay a stored input capsule by id; `--assert` checks deterministic replay.
    Replay {
        /// Capsule id from the replay-capsule store.
        capsule: String,
        /// Assert two independent replays match, including Trellis ledgers.
        #[arg(long = "assert")]
        assert_replay: bool,
        /// Write a Flight Recorder SerializedScenario trace JSON to this path.
        #[arg(long, value_name = "PATH")]
        export_trace: Option<PathBuf>,
    },
    /// Dry-run a fact against a surface via `tx.preview()` — nothing is applied.
    Simulate {
        /// The surface to simulate (`status` | `subscriptions`).
        #[arg(default_value = "status")]
        surface: String,
        /// Exact serde JSON for `InputFact`; preferred over the status shorthand.
        #[arg(long, value_name = "JSON")]
        fact: Option<String>,
        /// Session whose status the legacy shorthand applies to.
        #[arg(long)]
        session: Option<String>,
        /// Legacy status shorthand: distilled live-activity line to set.
        #[arg(long)]
        activity: Option<String>,
        /// Legacy status shorthand: distilled title to set.
        #[arg(long)]
        title: Option<String>,
        /// Unix **seconds** (NOT millis) — the reconciler's clock unit. When set,
        /// re-arms the TTL bucket (`now / refresh_secs`) exactly as a real distill
        /// would; passing millis lands in the wrong bucket and fabricates a
        /// spurious TTL refresh in the previewed plan. Omit to preview only the
        /// content change.
        #[arg(long)]
        now: Option<u64>,
    },
    /// Compare a live preview or replay capsule against a counterfactual fact.
    Diff {
        /// Surface hint (`status` | `subscriptions`); inferred from `--fact`.
        #[arg(default_value = "status")]
        surface: String,
        /// Exact serde JSON for `InputFact`.
        #[arg(long, value_name = "JSON")]
        fact: String,
        /// Optional replay capsule id; when set, replaces the capsule's last fact.
        #[arg(long)]
        capsule: Option<String>,
        /// Optional mutation fact for capsule mode; defaults to `--fact`.
        #[arg(long = "mutate-fact", value_name = "JSON")]
        mutate_fact: Option<String>,
    },
    /// Verify a live `why` cause by previewing cause-removed and unrelated facts.
    Acid {
        /// Handle to verify (`status:<session>` | `sub:<channel>`).
        handle: String,
        /// Exact serde JSON for `InputFact`.
        #[arg(long, value_name = "JSON")]
        fact: String,
        /// Specific cause label; defaults to the first live why input cause.
        #[arg(long)]
        cause: Option<String>,
    },
    /// Explain the latest change to a handle (`sub:<channel>` | `status:<session>` | `hook:<session>`).
    Why { handle: String },
    /// Live values for a surface (`status` | `subscriptions` | `session_start` | `hook_context`).
    State {
        surface: String,
        /// Surface-specific handle; for `hook_context`, this is the session id.
        handle: Option<String>,
        /// Include verbose graph debug output when supported by the surface.
        #[arg(long)]
        dump: bool,
    },
}

impl ProbeAction {
    /// Project the parsed verb into the `{verb, ...}` RPC params the daemon's
    /// `probe` arm dispatches on.
    fn to_rpc(&self) -> Result<(String, Value)> {
        match self {
            ProbeAction::Stats { surface, since } => Ok((
                "stats".into(),
                json!({ "verb": "stats", "surface": surface, "since": since }),
            )),
            ProbeAction::Oracle { surface, .. } => Ok((
                "oracle".into(),
                json!({ "verb": "oracle", "surface": surface }),
            )),
            ProbeAction::Seams => Ok(("seams".into(), json!({ "verb": "seams" }))),
            ProbeAction::Replay {
                capsule,
                assert_replay,
                export_trace,
            } => Ok((
                "replay".into(),
                json!({
                    "verb": "replay",
                    "capsule": capsule,
                    "assert": assert_replay,
                    "export_trace": export_trace.is_some(),
                }),
            )),
            ProbeAction::Simulate {
                surface,
                fact,
                session,
                activity,
                title,
                now,
            } => {
                let fact = fact
                    .as_deref()
                    .map(serde_json::from_str::<Value>)
                    .transpose()?;
                Ok((
                    "simulate".into(),
                    json!({ "verb": "simulate", "surface": surface, "fact": fact,
                            "session": session, "activity": activity, "title": title,
                            "now": now }),
                ))
            }
            ProbeAction::Diff {
                surface,
                fact,
                capsule,
                mutate_fact,
            } => {
                let fact = serde_json::from_str::<Value>(fact)?;
                let mutate_fact = mutate_fact
                    .as_deref()
                    .map(serde_json::from_str::<Value>)
                    .transpose()?;
                Ok((
                    "diff".into(),
                    json!({ "verb": "diff", "surface": surface, "fact": fact,
                            "capsule": capsule, "mutate_fact": mutate_fact }),
                ))
            }
            ProbeAction::Acid {
                handle,
                fact,
                cause,
            } => {
                let fact = serde_json::from_str::<Value>(fact)?;
                Ok((
                    "acid".into(),
                    json!({ "verb": "acid", "handle": handle, "fact": fact, "cause": cause }),
                ))
            }
            ProbeAction::Why { handle } => {
                Ok(("why".into(), json!({ "verb": "why", "handle": handle })))
            }
            ProbeAction::State {
                surface,
                handle,
                dump,
            } => Ok((
                "state".into(),
                json!({ "verb": "state", "surface": surface, "handle": handle, "dump": dump }),
            )),
        }
    }

    fn export_trace_path(&self) -> Option<PathBuf> {
        match self {
            ProbeAction::Replay { export_trace, .. } => export_trace.clone(),
            _ => None,
        }
    }
}

pub(in crate::cli) async fn probe(args: ProbeArgs) -> Result<()> {
    let export_trace_path = args.action.export_trace_path();
    let (verb, params) = args.action.to_rpc()?;
    let mut v = super::daemon_call_async("probe", params).await?;
    if let Some(path) = export_trace_path {
        let trace = v
            .get("trace_json")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("probe replay did not return trace_json"))?;
        std::fs::write(&path, trace)?;
        if let Some(obj) = v.as_object_mut() {
            obj.insert(
                "trace_path".into(),
                Value::String(path.display().to_string()),
            );
        }
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        print!("{}", render(&verb, &v));
    }
    Ok(())
}

/// Human view. `stats` gets its table here; the other verbs render in `render.rs`.
/// A wired-but-unimplemented shape (e.g. subscriptions simulate v2) prints its
/// marker; an unexpected shape falls back to a raw dump.
fn render(verb: &str, v: &Value) -> String {
    if v.get("implemented").and_then(Value::as_bool) == Some(false) {
        let msg = v
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("not implemented");
        return format!("probe {verb}: {msg}\n");
    }
    match verb {
        "stats" => stats_render::render_stats(v),
        "oracle" => render::render_oracle(v),
        "seams" => render::render_seams(v),
        "replay" => render::render_replay(v),
        "simulate" => render::render_simulate(v),
        "diff" => render::render_diff(v),
        "acid" => render::render_acid(v),
        "why" => render::render_why(v),
        "state" => state_render::render_state(v),
        _ => format!("{v}\n"),
    }
}

#[cfg(test)]
mod tests;
