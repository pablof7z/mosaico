use clap::{Args, Parser, Subcommand};

use super::admin::{AgentAction, AgentsAction, ChannelAction};
use super::config::ConfigArgs;
use super::debug::DebugAction;
use super::dispatch::DispatchArgs;
use super::harness::HarnessAction;
use super::install::InstallArgs;
use super::launch_cli::LaunchArgs;
use super::mcp::McpArgs;
use super::messaging::PublishArgs;
use super::my::MyAction;
use super::probe::ProbeArgs;
use super::pty::{PtyAction, PtySupervisorArgs};
use super::tui::TuiArgs;
use super::who::WhoArgs;

#[derive(Parser)]
#[command(
    name = "tenex-edge",
    about = "An identity and awareness fabric for the coding agents you already run."
)]
pub struct Cli {
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
    /// List agents currently visible in the workspace/channel.
    Who(WhoArgs),
    /// Read/send chat and manage NIP-29 channels (read, send, create, edit, list, init, join, leave, archive, switch).
    Channel {
        #[command(subcommand)]
        action: ChannelAction,
    },
    /// List invitable agents and prior sessions by npub.
    Agents {
        #[command(subcommand)]
        action: Option<AgentsAction>,
    },
    /// Manage local setup and operator-owned configuration.
    Mgmt {
        #[command(subcommand)]
        action: MgmtAction,
    },
    /// Start an agent session on a backend/workspace and hand it a message after ACK.
    Dispatch(DispatchArgs),
    /// Hook integration and statusline for any supported agent harness.
    Harness {
        #[command(subcommand)]
        action: HarnessAction,
    },
    /// Publish a long-form proposal (kind:30023) from this agent's session.
    Publish(PublishArgs),
    /// Launch an agent harness in a reattachable portable-pty session.
    Launch(LaunchArgs),
    /// Start an MCP server over stdio or HTTP.
    Mcp(McpArgs),
    /// Manage your own session title.
    My {
        #[command(subcommand)]
        action: MyAction,
    },
    /// Browse live sessions and attach to PTY-governed sessions.
    Tui(TuiArgs),
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
    /// Interactively configure model providers and role-to-model assignments.
    Config(ConfigArgs),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::session::SessionAction;
    use clap::{error::ErrorKind, Parser};

    fn parse_err(args: &[&str]) -> clap::Error {
        match Cli::try_parse_from(args) {
            Ok(_) => panic!("expected parse failure for {args:?}"),
            Err(err) => err,
        }
    }

    #[test]
    fn removed_tail_command_stays_unavailable() {
        let err = parse_err(&["tenex-edge", "tail", "--live"]);

        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn removed_chat_command_stays_unavailable() {
        let err = parse_err(&["tenex-edge", "chat", "read"]);

        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn removed_project_command_stays_unavailable() {
        let err = parse_err(&["tenex-edge", "project", "list"]);

        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn removed_agent_command_stays_unavailable() {
        let err = parse_err(&["tenex-edge", "agent", "list"]);

        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn removed_config_command_stays_unavailable() {
        let err = parse_err(&["tenex-edge", "config", "providers"]);

        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn removed_session_command_stays_unavailable() {
        let err = parse_err(&["tenex-edge", "session", "end", "--self"]);

        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn removed_channels_alias_stays_unavailable() {
        let err = parse_err(&["tenex-edge", "channels", "list"]);

        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn daemon_stop_parses() {
        let cli = Cli::try_parse_from(["tenex-edge", "daemon", "stop"]).unwrap();

        match cli.cmd {
            Cmd::Daemon(args) => assert!(matches!(args.action, Some(DaemonAction::Stop))),
            _ => panic!("expected daemon stop action"),
        }
    }

    #[test]
    fn daemon_restart_parses() {
        let cli = Cli::try_parse_from(["tenex-edge", "daemon", "restart"]).unwrap();

        match cli.cmd {
            Cmd::Daemon(args) => assert!(matches!(args.action, Some(DaemonAction::Restart))),
            _ => panic!("expected daemon restart action"),
        }
    }

    #[test]
    fn session_end_self_parses() {
        let cli = Cli::try_parse_from(["tenex-edge", "my", "session", "end", "--self"]).unwrap();
        match cli.cmd {
            Cmd::My {
                action:
                    MyAction::Session {
                        action: SessionAction::End(args),
                    },
            } => {
                assert!(args.self_session);
                assert!(args.session.is_none());
            }
            _ => panic!("expected my session end action"),
        }
    }

    #[test]
    fn mgmt_config_parses() {
        let cli = Cli::try_parse_from(["tenex-edge", "mgmt", "config", "providers"]).unwrap();
        assert!(matches!(cli.cmd, Cmd::Mgmt { .. }));
    }

    #[test]
    fn my_status_parses_with_topic() {
        let cli = Cli::try_parse_from([
            "tenex-edge",
            "my",
            "status",
            "--topic",
            "Researching MCP improvements around resource allocation",
        ])
        .unwrap();
        assert!(matches!(cli.cmd, Cmd::My { .. }));
    }

    #[test]
    fn mcp_command_parses() {
        let cli = Cli::try_parse_from(["tenex-edge", "mcp"]).unwrap();
        assert!(matches!(cli.cmd, Cmd::Mcp(_)));
    }

    #[test]
    fn mcp_http_command_parses() {
        let cli = Cli::try_parse_from(["tenex-edge", "mcp", "--http", "--port", "9000"]).unwrap();
        assert!(matches!(cli.cmd, Cmd::Mcp(_)));
    }

    #[test]
    fn tui_parses() {
        let cli = Cli::try_parse_from(["tenex-edge", "tui", "--refresh-secs", "3"]).unwrap();
        match cli.cmd {
            Cmd::Tui(args) => assert_eq!(args.refresh_secs, 3),
            _ => panic!("expected tui command"),
        }
    }
}
