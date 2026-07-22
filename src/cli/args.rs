use clap::{Args, Command, CommandFactory, Parser, Subcommand};

use super::admin::ChannelAction;
use super::agents::AgentsArgs;
use super::debug::DebugAction;
use super::dispatch::DispatchArgs;
use super::doctor::DoctorArgs;
use super::harness::HarnessAction;
use super::install::SetupArgs;
use super::mcp::McpArgs;
use super::messaging::WaitArgs;
use super::my::MyAction;
use super::pty::PtySupervisorArgs;
use super::relay::RelayArgs;
use super::uninstall::UninstallArgs;
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
    if !in_agent {
        cmd = cmd.after_help(
            "Run `mosaico` without a command to browse sessions and launchable agents together.\n\
             Run `mosaico <name>` to attach to a session or launch an agent directly.",
        );
    }
    let _ = cmd.print_help();
}

fn command_for_context(in_agent: bool) -> Command {
    let mut cmd = Cli::command();
    let visible: &[&str] = if in_agent {
        &["wait", "dispatch", "my", "doctor"]
    } else {
        &["who", "resume", "agents", "setup", "uninstall", "doctor"]
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
    about = "An identity and awareness fabric for the coding agents you already run.",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Show all commands, including hidden ones.
    #[arg(long, hide = true)]
    pub all: bool,

    /// Accept the current channel-topology suggestion using this child name and about.
    #[arg(
        long,
        num_args = 2,
        value_names = ["NEW-CHANNEL-NAME", "ABOUT"]
    )]
    pub(super) yes_lets_move: Option<Vec<String>>,

    #[command(subcommand)]
    pub(super) cmd: Option<Cmd>,
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
    /// Resume a session by its native Claude, Codex, Grok, Hermes, or OpenCode id.
    #[command(hide = true)]
    Resume(ResumeArgs),
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
    /// Diagnose Mosaico and safely repair local configuration and integrations.
    #[command(hide = true)]
    Doctor(DoctorArgs),
    /// Internal portable-pty supervisor process.
    #[command(name = "__pty-supervisor", hide = true)]
    PtySupervisor(PtySupervisorArgs),
    /// Configure Mosaico and install selected agent-harness integrations.
    #[command(hide = true)]
    Setup(SetupArgs),
    /// Remove Mosaico-owned integrations and optionally delete local state.
    #[command(hide = true)]
    Uninstall(UninstallArgs),
    /// Manage the per-machine daemon.
    #[command(name = "daemon")]
    Daemon(DaemonArgs),
    /// Run the bundled Croissant NIP-29 relay in the foreground.
    Relay(RelayArgs),
    /// Debug: drive a harness over the ACP / app-server transport end-to-end.
    #[command(name = "__acp-smoke", hide = true)]
    AcpSmoke(super::acp_smoke::AcpSmokeArgs),
    /// Attach to a matching session or launch a matching agent.
    #[command(external_subcommand)]
    Fallback(Vec<String>),
}

#[derive(Args)]
pub(super) struct ResumeArgs {
    /// Harness-native session identifier.
    pub(super) harness_id: String,
    /// Existing workspace path when native metadata has no usable cwd.
    #[arg(long, value_name = "PATH")]
    pub(super) workspace: Option<std::path::PathBuf>,
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
