//! Code-owned `(harness, transport)` capability table.
//!
//! This static table is the source of truth for every supported transport. It
//! supplies the executable, required environment, resume behavior, turn model,
//! and profile application for PTY, ACP, and app-server sessions.
//!
//! Invalid cells (e.g. Codex x Acp — Codex has no native ACP) simply have no
//! entry; `lookup` returns `None` and the caller fails loud.

use super::config::Transport;
use crate::session::Harness;

/// One row of the capability matrix.
#[derive(Debug)]
pub struct HarnessDriver {
    pub harness: Harness,
    pub transport: Transport,
    /// The launch prefix BEFORE user flags and profile flags. Authoritative
    /// here — never sniffed from argv[0] (claude-acp => the npx adapter, not
    /// the `claude` binary).
    pub base_argv: &'static [&'static str],
    /// Env the transport requires on the child (adapter wiring, hygiene).
    pub base_env: &'static [EnvDirective],
    pub resume: ResumeMechanism,
    pub steer: SteerPrimitive,
    pub turn: TurnModel,
    pub profile: ProfileMechanism,
}

/// A single env mutation applied to the child before launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvDirective {
    Set(&'static str, &'static str),
    Remove(&'static str),
}

/// How a session is re-entered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResumeMechanism {
    /// ACP `session/load {sessionId, cwd, mcpServers}`. Cross-process; also
    /// loads sessions made by non-ACP `opencode run`.
    AcpSessionLoad,
    /// Codex app-server `thread/resume` (or `thread/fork`) with the thread id.
    AppServerThreadResume,
    /// PTY: append `<flag> <id>` to argv. claude `--resume`,
    /// opencode `--session`, grok/hermes `--resume`.
    AppendFlag(&'static str),
    /// PTY: append fixed flags followed by the native id.
    AppendFlags(&'static [&'static str]),
    /// PTY: insert `<sub> <id>` right after argv[0]. codex `resume`.
    Subcommand(&'static str),
    /// Not resumable.
    None,
}

/// How mid-turn input is injected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SteerPrimitive {
    /// Codex app-server `turn/steer {threadId, expectedTurnId, input}`.
    AppServerSteer,
    /// Fire the harness's own hooks (settings.json hooks under ACP).
    Hooks,
    /// PTY bracketed-paste bytes via `pty::client::inject`.
    PtyPaste,
    None,
}

/// The turn/response model the transport exposes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnModel {
    /// Request/notification RPC: ACP `session/prompt`->stopReason, or codex
    /// `turn/start`->`turn/completed`.
    RpcTurn,
    /// Long-lived TTY; a turn is "text pasted + Enter", no completion signal.
    InteractivePty,
}

/// How an agent's optional harness-specific profile name is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileMechanism {
    /// Append the harness-native named-profile selector.
    CliFlag { flag: &'static str },
    /// Insert a top-level CLI profile selector immediately after argv[0].
    /// Hermes requires this placement before its `acp` subcommand.
    CliGlobalFlag { flag: &'static str },
    /// Codex app-server rejects `--profile`; compose the named config into an
    /// isolated CODEX_HOME instead.
    CodexAppServer,
    /// This transport cannot apply a named profile. `profile: null` remains valid.
    Unsupported,
}

static DRIVERS: &[HarnessDriver] = &[
    // ── Claude Code ───────────────────────────────────────────────
    HarnessDriver {
        harness: Harness::ClaudeCode,
        transport: Transport::Pty,
        base_argv: &["claude"],
        base_env: &[],
        resume: ResumeMechanism::AppendFlag("--resume"),
        steer: SteerPrimitive::PtyPaste,
        turn: TurnModel::InteractivePty,
        profile: ProfileMechanism::CliFlag { flag: "--agent" },
    },
    HarnessDriver {
        harness: Harness::ClaudeCode,
        transport: Transport::Acp,
        // adapter, not the claude binary — this is why base_argv must be
        // code-owned and not sniffed from argv[0].
        base_argv: &["npx", "--yes", "@agentclientprotocol/claude-agent-acp"],
        base_env: &[EnvDirective::Remove("CLAUDECODE")],
        resume: ResumeMechanism::AcpSessionLoad,
        steer: SteerPrimitive::Hooks,
        turn: TurnModel::RpcTurn,
        profile: ProfileMechanism::Unsupported,
    },
    // ── Codex ─────────────────────────────────────────────────────
    HarnessDriver {
        harness: Harness::Codex,
        transport: Transport::AppServer,
        base_argv: &["codex", "app-server"],
        base_env: &[],
        resume: ResumeMechanism::AppServerThreadResume,
        steer: SteerPrimitive::AppServerSteer,
        turn: TurnModel::RpcTurn,
        profile: ProfileMechanism::CodexAppServer,
    },
    HarnessDriver {
        harness: Harness::Codex,
        transport: Transport::Pty,
        base_argv: &["codex"],
        base_env: &[],
        resume: ResumeMechanism::Subcommand("resume"),
        steer: SteerPrimitive::PtyPaste,
        turn: TurnModel::InteractivePty,
        profile: ProfileMechanism::CliFlag { flag: "--profile" },
    },
    // ── OpenCode ──────────────────────────────────────────────────
    HarnessDriver {
        harness: Harness::Opencode,
        transport: Transport::Acp,
        base_argv: &["opencode", "acp"],
        base_env: &[],
        resume: ResumeMechanism::AcpSessionLoad,
        steer: SteerPrimitive::None,
        turn: TurnModel::RpcTurn,
        // session/new agent params are ignored by current OpenCode ACP.
        profile: ProfileMechanism::Unsupported,
    },
    HarnessDriver {
        harness: Harness::Opencode,
        transport: Transport::Pty,
        base_argv: &["opencode"],
        base_env: &[],
        resume: ResumeMechanism::AppendFlag("--session"),
        steer: SteerPrimitive::PtyPaste,
        turn: TurnModel::InteractivePty,
        profile: ProfileMechanism::CliFlag { flag: "--agent" },
    },
    // ── Grok (PTY only, profile unknown) ──────────────────────────
    HarnessDriver {
        harness: Harness::Grok,
        transport: Transport::Pty,
        base_argv: &["grok"],
        base_env: &[],
        resume: ResumeMechanism::AppendFlag("--resume"),
        steer: SteerPrimitive::PtyPaste,
        turn: TurnModel::InteractivePty,
        profile: ProfileMechanism::Unsupported,
    },
    // ── Goose ─────────────────────────────────────────────────────
    HarnessDriver {
        harness: Harness::Goose,
        transport: Transport::Acp,
        base_argv: &["goose", "acp"],
        base_env: &[],
        resume: ResumeMechanism::AcpSessionLoad,
        steer: SteerPrimitive::None,
        turn: TurnModel::RpcTurn,
        profile: ProfileMechanism::Unsupported,
    },
    HarnessDriver {
        harness: Harness::Goose,
        transport: Transport::Pty,
        base_argv: &["goose", "session"],
        base_env: &[],
        resume: ResumeMechanism::AppendFlags(&["--resume", "--session-id"]),
        steer: SteerPrimitive::PtyPaste,
        turn: TurnModel::InteractivePty,
        profile: ProfileMechanism::Unsupported,
    },
    // ── Hermes ───────────────────────────────────────────────────────────
    HarnessDriver {
        harness: Harness::Hermes,
        transport: Transport::Acp,
        base_argv: &["hermes", "acp"],
        base_env: &[],
        resume: ResumeMechanism::AcpSessionLoad,
        steer: SteerPrimitive::None,
        turn: TurnModel::RpcTurn,
        profile: ProfileMechanism::CliGlobalFlag { flag: "--profile" },
    },
    HarnessDriver {
        harness: Harness::Hermes,
        transport: Transport::Pty,
        base_argv: &["hermes"],
        base_env: &[],
        resume: ResumeMechanism::AppendFlag("--resume"),
        steer: SteerPrimitive::PtyPaste,
        turn: TurnModel::InteractivePty,
        profile: ProfileMechanism::CliGlobalFlag { flag: "--profile" },
    },
];

/// Look up the driver for a `(harness, transport)` pair. `None` for invalid
/// cells (caller fails loud).
pub fn lookup(harness: Harness, transport: Transport) -> Option<&'static HarnessDriver> {
    DRIVERS
        .iter()
        .find(|d| d.harness == harness && d.transport == transport)
}

/// All rows (for enumeration/tests).
pub fn all() -> &'static [HarnessDriver] {
    DRIVERS
}
