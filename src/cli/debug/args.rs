use crate::cli::explain::{explain, ExplainArgs};
use anyhow::Result;
use clap::Subcommand;
use std::time::Duration;

#[derive(Subcommand)]
pub(in crate::cli) enum DebugAction {
    /// Explain a published artifact using its reconciler receipt.
    Explain(ExplainArgs),
    /// Live TUI for hook injections and mosaico command invocations.
    HookTail {
        /// Filter panes/events to one or more workspaces (repeatable).
        #[arg(long = "workspace", value_name = "WORKSPACE")]
        workspaces: Vec<String>,
        /// Filter panes/events to a public session npub, hex pubkey, or handle.
        #[arg(long)]
        session: Option<String>,
        /// Maximum panes in the grid.
        #[arg(long, default_value = "6")]
        panes: usize,
        /// Refresh interval in milliseconds.
        #[arg(long, default_value = "1000")]
        refresh_ms: u64,
    },
}

pub(in crate::cli) async fn debug(action: DebugAction) -> Result<()> {
    match action {
        DebugAction::Explain(args) => explain(args),
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
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    #[test]
    fn removed_outbox_command_stays_unavailable() {
        assert!(crate::cli::args::Cli::try_parse_from(["mosaico", "debug", "outbox"]).is_err());
    }
}
