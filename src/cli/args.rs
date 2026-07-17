use clap::{Args, Command, CommandFactory, Parser, Subcommand};

use super::admin::ChannelAction;
use super::agents::AgentsArgs;
use super::debug::DebugAction;
use super::dispatch::DispatchArgs;
use super::harness::HarnessAction;
use super::install::InstallArgs;
use super::mcp::McpArgs;
use super::messaging::WaitArgs;
use super::my::MyAction;
use super::pty::PtySupervisorArgs;
use super::who::WhoArgs;

/// Print the top-level help with every hidden subcommand unhidden, for
/// `mosaico --help --all`. Recursively clears `hide` on subcommands only;
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

/// Print top-level help for the current caller context. Human operators see
/// their local-management commands; agents see their self-service commands.
/// Internal/debug commands remain hidden in both contexts; use `--help --all`
/// to see everything.
pub fn print_help_contextual() {
    let in_agent = super::agent_env_slug().is_some();
    let mut cmd = command_for_context(in_agent);
    let _ = cmd.print_help();
}

fn command_for_context(in_agent: bool) -> Command {
    let mut cmd = Cli::command();
    let visible: &[&str] = if in_agent {
        &["wait", "dispatch", "my"]
    } else {
        &["who", "sessions", "agents"]
    };
    for sub in cmd.get_subcommands_mut() {
        if visible.contains(&sub.get_name()) {
            let owned = std::mem::take(sub);
            *sub = owned.hide(false);
        }
    }
    cmd
}

#[derive(Parser)]
#[command(
    name = "mosaico",
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
    #[command(hide = true)]
    Who(WhoArgs),
    /// Select, attach to, or immediately kill local agent sessions.
    #[command(hide = true)]
    Sessions,
    /// Read/send chat and manage channels (read, send, create, edit, list, init, join, leave, archive, switch).
    Channel {
        #[command(subcommand)]
        action: ChannelAction,
    },
    /// Block until matching chat arrives or the timeout passes.
    #[command(hide = true)]
    Wait(WaitArgs),
    /// Launch, configure, or remove local agents and harness profiles.
    #[command(hide = true)]
    Agents(AgentsArgs),
    /// Start an agent session on a backend/workspace and hand it a message after ACK.
    #[command(hide = true)]
    Dispatch(DispatchArgs),
    /// Hook integration and statusline for any supported agent harness.
    #[command(hide = true)]
    Harness {
        #[command(subcommand)]
        action: HarnessAction,
    },
    /// Start an MCP server over stdio or HTTP.
    Mcp(McpArgs),
    /// Inspect and manage your own session.
    #[command(hide = true)]
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
    /// Internal portable-pty supervisor process.
    #[command(name = "__pty-supervisor", hide = true)]
    PtySupervisor(PtySupervisorArgs),
    /// Detect local agent harnesses and wire mosaico's hook entries into each.
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

#[cfg(test)]
#[path = "args/tests.rs"]
mod tests;
