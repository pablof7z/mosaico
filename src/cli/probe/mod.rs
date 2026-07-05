//! `tenex-edge probe` — hidden diagnostic surface over the reconciler frontier
//! (frontier design §4). A thin client: it forwards a verb + params to the
//! daemon's `probe` RPC and renders the JSON it returns. `--json` (global) emits
//! the raw daemon JSON instead of the human view.
//!
//! Verbs: `stats` (aggregate value evidence, §4.1), `oracle` (live
//! incremental-equals-full correctness, §4.6), `seams` (frontier modes, §4.5),
//! `simulate` (dry-run a fact via `tx.preview()`, the keystone, §3), `why` (live
//! causality for a handle, §4.3), and `state` (live per-surface values, §4.3).

mod render;
mod stats_render;

use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{json, Value};

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
        /// One surface (`status` | `subscriptions` | `hook_context`); omit for all.
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
    /// Dry-run a fact against a surface via `tx.preview()` — nothing is applied.
    Simulate {
        /// The surface to simulate (`status`; `subscriptions` is a v2 follow-up).
        #[arg(default_value = "status")]
        surface: String,
        /// Session whose status the fact applies to.
        #[arg(long)]
        session: String,
        /// The distilled live-activity line the fact would set.
        #[arg(long)]
        activity: Option<String>,
        /// The distilled title the fact would set.
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
    /// Explain the latest change to a handle (`sub:<channel>` | `status:<session>`).
    Why { handle: String },
    /// Live values for a surface (`status` | `subscriptions` | `hook_context`).
    State { surface: String },
}

impl ProbeAction {
    /// Project the parsed verb into the `{verb, ...}` RPC params the daemon's
    /// `probe` arm dispatches on.
    fn to_rpc(&self) -> (String, Value) {
        match self {
            ProbeAction::Stats { surface, since } => (
                "stats".into(),
                json!({ "verb": "stats", "surface": surface, "since": since }),
            ),
            ProbeAction::Oracle { surface, .. } => (
                "oracle".into(),
                json!({ "verb": "oracle", "surface": surface }),
            ),
            ProbeAction::Seams => ("seams".into(), json!({ "verb": "seams" })),
            ProbeAction::Simulate {
                surface,
                session,
                activity,
                title,
                now,
            } => (
                "simulate".into(),
                json!({ "verb": "simulate", "surface": surface, "session": session,
                        "activity": activity, "title": title, "now": now }),
            ),
            ProbeAction::Why { handle } => {
                ("why".into(), json!({ "verb": "why", "handle": handle }))
            }
            ProbeAction::State { surface } => (
                "state".into(),
                json!({ "verb": "state", "surface": surface }),
            ),
        }
    }
}

pub(in crate::cli) async fn probe(args: ProbeArgs) -> Result<()> {
    let (verb, params) = args.action.to_rpc();
    let v = super::daemon_call_async("probe", params).await?;
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
        "simulate" => render::render_simulate(v),
        "why" => render::render_why(v),
        "state" => render::render_state(v),
        _ => format!("{v}\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unimplemented_shape_renders_marker() {
        let v = json!({ "verb": "simulate", "implemented": false,
                        "message": "subscriptions simulate is a v2 follow-up" });
        let text = render("simulate", &v);
        assert_eq!(
            text,
            "probe simulate: subscriptions simulate is a v2 follow-up\n"
        );
    }

    #[test]
    fn stats_action_projects_rpc_params() {
        let action = ProbeAction::Stats {
            surface: Some("status".into()),
            since: 42,
        };
        let (verb, params) = action.to_rpc();
        assert_eq!(verb, "stats");
        assert_eq!(params["verb"], "stats");
        assert_eq!(params["surface"], "status");
        assert_eq!(params["since"], 42);
    }

    #[test]
    fn seams_action_projects_rpc_params() {
        let (verb, params) = ProbeAction::Seams.to_rpc();
        assert_eq!(verb, "seams");
        assert_eq!(params["verb"], "seams");
    }

    #[test]
    fn simulate_action_projects_rpc_params() {
        let action = ProbeAction::Simulate {
            surface: "status".into(),
            session: "s1".into(),
            activity: Some("reviewing the PR".into()),
            title: None,
            now: None,
        };
        let (verb, params) = action.to_rpc();
        assert_eq!(verb, "simulate");
        assert_eq!(params["session"], "s1");
        assert_eq!(params["activity"], "reviewing the PR");
        assert!(params["title"].is_null());
    }
}
