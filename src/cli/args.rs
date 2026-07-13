use clap::{Args, Command, CommandFactory, Parser, Subcommand};

use super::admin::{AgentAction, ChannelAction};
use super::config::ConfigArgs;
use super::debug::DebugAction;
use super::dispatch::DispatchArgs;
use super::harness::HarnessAction;
use super::install::InstallArgs;
use super::launch_cli::LaunchArgs;
use super::mcp::McpArgs;
use super::messaging::{PublishArgs, WaitArgs};
use super::my::MyAction;
use super::probe::ProbeArgs;
use super::pty::{PtyAction, PtySupervisorArgs};
use super::who::WhoArgs;

/// Print the top-level help with every hidden subcommand unhidden, for
/// `tenex-edge --help --all`. Recursively clears `hide` on subcommands only;
/// hidden args stay hidden (they are internal modifiers).
pub fn print_help_all() {
    let mut cmd = Cli::command();
    unhide_subcommands(&mut cmd);
    let _ = cmd.print_help();
}

fn unhide_subcommands(cmd: &mut Command) {
    for sub in cmd.get_subcommands_mut() {
        let owned = std::mem::take(sub);
        *sub = owned.hide(false);
        unhide_subcommands(sub);
    }
}

#[derive(Parser)]
#[command(
    name = "tenex-edge",
    about = "An identity and awareness fabric for the coding agents you already run."
)]
pub struct Cli {
    /// Show all commands, including hidden ones.
    #[arg(long, hide = true)]
    pub all: bool,

    #[command(subcommand)]
    pub(super) cmd: Cmd,
}

#[derive(Subcommand)]
pub(super) enum Cmd {
    // session-start / turn-start / turn-check / turn-end are NOT
    // subcommands. They are hook-driven lifecycle steps invoked only through
    // `harness <name> hook --type <…>`, which parses the harness's stdin payload and calls the
    // corresponding private fn (session_start_inner / turn_start / turn_check /
    // turn_end). Session end has a small public surface for agents to end
    // themselves explicitly.
    /// Show the human/operator fabric view.
    Who(WhoArgs),
    /// Read/send chat and manage NIP-29 channels (read, send, create, edit, list, init, join, leave, archive, switch).
    Channel {
        #[command(subcommand)]
        action: ChannelAction,
    },
    /// Block until matching chat arrives or the timeout passes.
    Wait(WaitArgs),
    /// Manage local setup and operator-owned configuration.
    Mgmt {
        #[command(subcommand)]
        action: MgmtAction,
    },
    /// Start an agent session on a backend/workspace and hand it a message after ACK.
    Dispatch(DispatchArgs),
    /// Hook integration and statusline for any supported agent harness.
    #[command(hide = true)]
    Harness {
        #[command(subcommand)]
        action: HarnessAction,
    },
    /// Publish a long-form proposal (kind:30023) from this agent's session.
    Publish(PublishArgs),
    /// Launch an agent through an attachable PTY or a headless RPC transport.
    Launch(LaunchArgs),
    /// Start an MCP server over stdio or HTTP.
    Mcp(McpArgs),
    /// Inspect and manage your own session.
    My {
        #[command(subcommand)]
        action: MyAction,
    },
    /// Local debugging tools for hook injection and command telemetry.
    #[command(hide = true)]
    Debug {
        #[command(subcommand)]
        action: DebugAction,
    },
    /// Diagnostic probe over the reconciler frontier: stats/oracle/simulate/why/state.
    #[command(hide = true)]
    Probe(ProbeArgs),
    /// Experimental portable-pty supervisor test surface.
    #[command(hide = true)]
    Pty {
        #[command(subcommand)]
        action: PtyAction,
    },
    /// Internal portable-pty supervisor process.
    #[command(name = "__pty-supervisor", hide = true)]
    PtySupervisor(PtySupervisorArgs),
    /// Detect local agent harnesses and wire tenex-edge's hook entries into each.
    #[command(hide = true)]
    Install(InstallArgs),
    /// Manage the per-machine daemon.
    #[command(name = "daemon")]
    Daemon(DaemonArgs),
    /// Debug: drive a harness over the ACP / app-server transport end-to-end.
    #[command(name = "__acp-smoke", hide = true)]
    AcpSmoke(super::acp_smoke::AcpSmokeArgs),
}

#[derive(Args)]
pub(super) struct DaemonArgs {
    #[command(subcommand)]
    pub(super) action: Option<DaemonAction>,
}

#[derive(Subcommand)]
pub(super) enum DaemonAction {
    /// Restart the daemon while preserving detached agent sessions.
    Restart,
    /// Stop the daemon and prevent hooks from restarting it.
    Stop,
}

#[derive(Subcommand)]
pub(super) enum MgmtAction {
    /// Manage the local agent keystore: agents that have a private key on THIS
    /// machine under `<edge_home>/agents/<slug>.json`. These are the identities
    /// you can spawn locally; channel membership is governed separately by the
    /// codec (e.g. the NIP-29 group's member list), not here.
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// Inspect and control sessions hosted by this machine.
    Session {
        #[command(subcommand)]
        action: MgmtSessionAction,
    },
    /// Interactively configure model providers and role-to-model assignments.
    Config(ConfigArgs),
}

#[derive(Subcommand)]
pub(super) enum MgmtSessionAction {
    /// Select and kill local sessions from an interactive checklist.
    List,
}

#[cfg(test)]
#[path = "args/tests.rs"]
mod tests;
