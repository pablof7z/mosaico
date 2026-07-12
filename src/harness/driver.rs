//! Code-owned `(harness, transport)` capability table.
//!
//! This static table is the source of truth for the **RPC transports it actually
//! drives** — `Acp` / `AppServer` — supplying their `base_argv`, `base_env`,
//! `resume`, `steer`, `turn`, and `profile` when `harness::resolve` plans an
//! ACP/app-server launch. It is NOT (yet) the source of truth for driving PTY or
//! headless sessions: PTY resume and headless exec are still shaped by the argv[0]
//! sniffers in `session_host::registry::{resume_shape_for_bin,
//! headless_shape_for_bin}`, which remain authoritative for those paths. The
//! `Pty` / `HeadlessExec` rows here are therefore consulted only by
//! `harness::resolve`'s built-in-PTY fallback (and capability enumeration); their
//! `resume`/`steer`/`turn` fields do NOT drive a real session and duplicate the
//! sniffers' knowledge — do not treat them as the drive path. Migrating PTY/
//! headless onto this table (retiring the sniffers) is deliberately left as future
//! work.
//!
//! Invalid cells (e.g. Codex x Acp — Codex has no native ACP) simply have no
//! entry; `lookup` returns `None` and the caller fails loud.

use super::config::Transport;
use crate::session::Harness;

/// One row of the capability matrix.
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
    /// PTY/exec: append `<flag> <id>` to argv. claude `--resume`,
    /// opencode `--session`, grok `--resume`.
    AppendFlag(&'static str),
    /// PTY: insert `<sub> <id>` right after argv[0]. codex `resume`.
    Subcommand(&'static str),
    /// Headless: replay the recorded native id through the harness's own resume
    /// path (`codex exec resume <id>`, `opencode run --session <id>`).
    ExecReplay,
    /// Not resumable.
    None,
}

/// How mid-turn input is injected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SteerPrimitive {
    /// Codex app-server `turn/steer {threadId, expectedTurnId, input}`.
    AppServerSteer,
    /// Fire the harness's own hooks (settings.json hooks under ACP/exec).
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
    /// Process runs to exit; stdout is the whole turn.
    OneShot,
}

/// How a bundle's `profile` object is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileMechanism {
    /// codex: each top-level profile entry -> `<flag> key=value` launch args.
    CliConfigFlags { flag: &'static str },
    /// claude/opencode: materialize the profile object into a per-session
    /// scratch settings file and point the child at it (never mutate the repo).
    CwdSettingsFile { relpath: &'static str },
    /// No profile support; a non-empty `profile` is a hard config error.
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
        profile: ProfileMechanism::CwdSettingsFile {
            relpath: ".claude/settings.json",
        },
    },
    HarnessDriver {
        harness: Harness::ClaudeCode,
        transport: Transport::Acp,
        // adapter, not the claude binary — this is why base_argv must be
        // code-owned and not sniffed from argv[0].
        base_argv: &["npx", "@zed-industries/claude-code-acp"],
        base_env: &[EnvDirective::Remove("CLAUDECODE")],
        resume: ResumeMechanism::AcpSessionLoad,
        steer: SteerPrimitive::Hooks,
        turn: TurnModel::RpcTurn,
        profile: ProfileMechanism::CwdSettingsFile {
            relpath: ".claude/settings.json",
        },
    },
    HarnessDriver {
        harness: Harness::ClaudeCode,
        transport: Transport::HeadlessExec,
        base_argv: &["claude"],
        base_env: &[],
        resume: ResumeMechanism::AppendFlag("--resume"),
        steer: SteerPrimitive::None,
        turn: TurnModel::OneShot,
        profile: ProfileMechanism::CwdSettingsFile {
            relpath: ".claude/settings.json",
        },
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
        profile: ProfileMechanism::CliConfigFlags { flag: "-c" },
    },
    HarnessDriver {
        harness: Harness::Codex,
        transport: Transport::Pty,
        base_argv: &["codex"],
        base_env: &[],
        resume: ResumeMechanism::Subcommand("resume"),
        steer: SteerPrimitive::PtyPaste,
        turn: TurnModel::InteractivePty,
        profile: ProfileMechanism::CliConfigFlags { flag: "-c" },
    },
    HarnessDriver {
        harness: Harness::Codex,
        transport: Transport::HeadlessExec,
        base_argv: &["codex"],
        base_env: &[],
        resume: ResumeMechanism::ExecReplay,
        steer: SteerPrimitive::None,
        turn: TurnModel::OneShot,
        profile: ProfileMechanism::CliConfigFlags { flag: "-c" },
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
        // session/new model/agent params are IGNORED -> must go through a file.
        profile: ProfileMechanism::CwdSettingsFile {
            relpath: "opencode.json",
        },
    },
    HarnessDriver {
        harness: Harness::Opencode,
        transport: Transport::Pty,
        base_argv: &["opencode"],
        base_env: &[],
        resume: ResumeMechanism::AppendFlag("--session"),
        steer: SteerPrimitive::PtyPaste,
        turn: TurnModel::InteractivePty,
        profile: ProfileMechanism::CwdSettingsFile {
            relpath: "opencode.json",
        },
    },
    HarnessDriver {
        harness: Harness::Opencode,
        transport: Transport::HeadlessExec,
        base_argv: &["opencode", "run", "--format", "json"],
        base_env: &[],
        resume: ResumeMechanism::AppendFlag("--session"),
        steer: SteerPrimitive::None,
        turn: TurnModel::OneShot,
        profile: ProfileMechanism::CwdSettingsFile {
            relpath: "opencode.json",
        },
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
