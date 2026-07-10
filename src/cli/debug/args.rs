use crate::cli::admin::doctor;
use crate::cli::explain::{explain, ExplainArgs};
use crate::cli::validate::{validate, ValidateArgs};
use anyhow::Result;
use clap::Subcommand;
use std::time::Duration;

#[derive(Subcommand)]
pub(in crate::cli) enum DebugAction {
    /// Diagnose daemon relay and storage-path configuration.
    Doctor,
    /// Explain a published artifact: the reconciler receipt + the exact LLM
    /// inputs (system prompt, transcript slice, model, raw response) behind it.
    Explain(ExplainArgs),
    /// Validate a surface, handle, event/message/recipient target, awareness
    /// target, channel/readiness/readiness_attempt target, commit target, fact,
    /// or replay capsule with explanations.
    Validate(ValidateArgs),
    /// Live TUI for hook injections and tenex-edge command invocations.
    HookTail {
        /// Filter panes/events to one or more workspaces (repeatable).
        #[arg(long = "workspace", alias = "root", value_name = "WORKSPACE")]
        workspaces: Vec<String>,
        /// Filter panes/events to a session id (or a unique prefix of it).
        #[arg(long)]
        session: Option<String>,
        /// Maximum panes in the grid.
        #[arg(long, default_value = "6")]
        panes: usize,
        /// Refresh interval in milliseconds.
        #[arg(long, default_value = "1000")]
        refresh_ms: u64,
    },
    /// Inspect the status publish outbox.
    Outbox {
        /// Keep printing the outbox state until interrupted.
        #[arg(long)]
        live: bool,
        /// Maximum rows to show.
        #[arg(long, default_value = "50")]
        limit: u64,
        /// Refresh interval in milliseconds when --live is set.
        #[arg(long, default_value = "1000")]
        refresh_ms: u64,
    },
}

pub(in crate::cli) async fn debug(action: DebugAction) -> Result<()> {
    match action {
        DebugAction::Doctor => doctor().await,
        DebugAction::Explain(args) => explain(args),
        DebugAction::Validate(args) => validate(args).await,
        DebugAction::HookTail {
            workspaces,
            session,
            panes,
            refresh_ms,
        } => super::hook_tail(super::HookTailOpts {
            roots: workspaces,
            session,
            panes,
            refresh: Duration::from_millis(refresh_ms.max(100)),
        }),
        DebugAction::Outbox {
            live,
            limit,
            refresh_ms,
        } => super::outbox(live, limit, Duration::from_millis(refresh_ms.max(100))).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn debug_outbox_defaults_are_owned_by_debug_args() {
        let cli = crate::cli::args::Cli::try_parse_from(["tenex-edge", "debug", "outbox"])
            .expect("debug outbox parses");

        match cli.cmd {
            crate::cli::args::Cmd::Debug {
                action:
                    DebugAction::Outbox {
                        live,
                        limit,
                        refresh_ms,
                    },
            } => {
                assert!(!live);
                assert_eq!(limit, 50);
                assert_eq!(refresh_ms, 1_000);
            }
            _ => panic!("expected debug outbox command"),
        }
    }
}
