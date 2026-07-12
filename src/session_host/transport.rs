//! Transport-agnostic session hosting seam.
//!
//! A "hosted session" is opened over exactly one transport. Today that is
//! always the portable PTY (`PtyTransport`, byte-identical to the current
//! `open_agent_session` path). `AcpTransport` adds a stdio JSON-RPC backend
//! (ACP / codex app-server) for harnesses that expose an `RpcTurn` model.
//!
//! Object dispatch is via the [`TransportImpl`] enum rather than `dyn` (native
//! async-fn-in-trait is not object-safe and we avoid pulling `async-trait`).
//! The [`SessionTransport`] trait documents the contract each backend fulfils.

pub mod acp;
mod acp_runtime;
pub mod pty;

use anyhow::Result;

use crate::harness::{self, config::HarnessesConfig, Transport};

/// Which transport hosts a session. Stringifies to `"pty"` / `"acp"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    Pty,
    Acp,
}

impl TransportKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TransportKind::Pty => "pty",
            TransportKind::Acp => "acp",
        }
    }
}

/// Fully-resolved, transport-agnostic launch intent.
#[derive(Debug, Clone)]
pub struct LaunchSpec {
    pub slug: String,
    pub root: String,
    pub abs_path: String,
    pub group: Option<String>,
    pub ephemeral: bool,
    /// Resolved argv incl. base_argv + profile + user flags + agent-def args.
    pub base_command: Vec<String>,
}

/// A prior session's native resume token.
#[derive(Debug, Clone)]
pub struct ResumeSpec {
    pub native_id: String,
}

/// What the daemon needs after a session opens. For PTY this carries a real
/// `LaunchMetadata` so `bootstrap_pty_session_start` stays unchanged.
#[derive(Debug)]
pub struct SessionEndpoint {
    pub kind: TransportKind,
    pub endpoint_id: String,
    pub watch_pid: Option<i32>,
    pub meta: crate::pty::LaunchMetadata,
}

/// A live-session address the daemon holds after registration.
#[derive(Debug, Clone)]
pub struct EndpointRef {
    pub kind: TransportKind,
    pub endpoint_id: String,
}

/// The transport contract. Each method maps to an existing daemon behavior;
/// the two backends fulfil them by fundamentally different mechanisms
/// (relaunch-with-flag + bracketed-paste vs. JSON-RPC).
#[allow(async_fn_in_trait)]
pub trait SessionTransport {
    fn kind(&self) -> TransportKind;

    /// Open a brand-new harness session.
    async fn launch(&self, spec: &LaunchSpec) -> Result<SessionEndpoint>;

    /// Reopen a prior session by its native token.
    async fn resume(&self, spec: &LaunchSpec, resume: &ResumeSpec) -> Result<SessionEndpoint>;

    /// Deliver text; `submit` completes the turn.
    async fn deliver(&self, ep: &EndpointRef, text: &str, submit: bool) -> Result<()>;

    fn is_live(&self, ep: &EndpointRef) -> bool;

    async fn kill(&self, ep: &EndpointRef) -> Result<()>;
}

pub use acp::AcpTransport;
pub use pty::PtyTransport;

/// Enum dispatcher giving dynamic transport selection without `dyn`.
pub enum TransportImpl {
    Pty(PtyTransport),
    Acp(AcpTransport),
}

impl TransportImpl {
    pub fn kind(&self) -> TransportKind {
        match self {
            TransportImpl::Pty(t) => t.kind(),
            TransportImpl::Acp(t) => t.kind(),
        }
    }

    pub async fn launch(&self, spec: &LaunchSpec) -> Result<SessionEndpoint> {
        match self {
            TransportImpl::Pty(t) => t.launch(spec).await,
            TransportImpl::Acp(t) => t.launch(spec).await,
        }
    }

    pub async fn resume(&self, spec: &LaunchSpec, resume: &ResumeSpec) -> Result<SessionEndpoint> {
        match self {
            TransportImpl::Pty(t) => t.resume(spec, resume).await,
            TransportImpl::Acp(t) => t.resume(spec, resume).await,
        }
    }

    pub async fn deliver(&self, ep: &EndpointRef, text: &str, submit: bool) -> Result<()> {
        match self {
            TransportImpl::Pty(t) => t.deliver(ep, text, submit).await,
            TransportImpl::Acp(t) => t.deliver(ep, text, submit).await,
        }
    }

    pub fn is_live(&self, ep: &EndpointRef) -> bool {
        match self {
            TransportImpl::Pty(t) => t.is_live(ep),
            TransportImpl::Acp(t) => t.is_live(ep),
        }
    }

    pub async fn kill(&self, ep: &EndpointRef) -> Result<()> {
        match self {
            TransportImpl::Pty(t) => t.kill(ep).await,
            TransportImpl::Acp(t) => t.kill(ep).await,
        }
    }
}

/// Pick the transport for an agent from its configured harness bundle.
///
/// `bundle` is the agent's `harness` field (a `harnesses.json` bundle name), or
/// `None` when the agent has no bundle configured — in which case the transport
/// is always the PTY, preserving current behavior byte-for-byte. A bundle whose
/// transport is `Acp`/`AppServer` selects [`AcpTransport`]; anything else
/// (`Pty`/`HeadlessExec`) selects [`PtyTransport`].
///
/// Defect #3 contract: this launch-time entry point **fails open to the PTY**
/// (with a loud `WARN`) when `harnesses.json` is missing or malformed, rather
/// than aborting a launch that previously worked. A configured-but-unresolvable
/// bundle is almost always a corrupt/edited config, and a bundle-carrying agent
/// that used to launch on the PTY (its bundle resolving to `Pty`) must not be
/// bricked by an unrelated config error. This mirrors the `agent_harness_bundle`
/// "absent config => `None` => PTY" fail-open. The pure core
/// [`select_transport_with`] / [`transport_kind_for`] stay fail-loud so the
/// strict resolution contract remains unit-testable; only this IO wrapper softens
/// it. The cost — a genuinely-ACP agent silently launching on the PTY under a
/// corrupt config — surfaces loudly downstream (an ACP-only agent has no PTY
/// `commands` entry, so the PTY launch then fails at command resolution).
pub fn select_transport(bundle: Option<&str>) -> Result<TransportImpl> {
    Ok(match resolve_kind_fail_open(bundle) {
        TransportKind::Acp => TransportImpl::Acp(AcpTransport),
        TransportKind::Pty => TransportImpl::Pty(PtyTransport),
    })
}

/// Resolve the transport kind for an agent `slug` from its configured harness
/// bundle, failing open to [`TransportKind::Pty`] on any resolution error
/// (mirrors [`select_transport`]). Used by the transport-aware delivery path,
/// which must classify a live session's endpoint as PTY vs. ACP.
pub fn transport_kind_for_slug(slug: &str) -> TransportKind {
    let bundle = crate::identity::agent_harness_bundle(&crate::config::edge_home(), slug);
    resolve_kind_fail_open(bundle.as_deref())
}

/// Shared fail-open resolution: `None`/empty bundle => PTY (no config touched);
/// otherwise load `harnesses.json` and resolve the kind, falling back to PTY with
/// a `WARN` if the config is missing/malformed or the bundle is unresolvable.
fn resolve_kind_fail_open(bundle: Option<&str>) -> TransportKind {
    // Short-circuit the no-bundle case WITHOUT touching harnesses.json: an agent
    // with no configured bundle always launches on the PTY, and must not be made
    // to depend on (or fail because of) a malformed harnesses.json it never uses.
    let Some(bundle) = bundle.filter(|b| !b.is_empty()) else {
        return TransportKind::Pty;
    };
    resolve_kind_fail_open_with(bundle, HarnessesConfig::load())
}

/// Testable core of [`resolve_kind_fail_open`]: given a non-empty bundle and the
/// (possibly failed) config load, resolve the kind, failing open to PTY with a
/// `WARN` on any error (defect #3). Kept separate from the IO so the fail-open
/// contract is unit-testable without touching the filesystem/`edge_home`.
fn resolve_kind_fail_open_with(
    bundle: &str,
    cfg: anyhow::Result<HarnessesConfig>,
) -> TransportKind {
    match cfg.and_then(|cfg| transport_kind_for(&cfg, Some(bundle))) {
        Ok(kind) => kind,
        Err(e) => {
            tracing::warn!(
                bundle = %bundle,
                error = %format!("{e:#}"),
                "harness bundle transport resolution failed (missing/malformed harnesses.json?); \
                 falling back to PTY transport"
            );
            TransportKind::Pty
        }
    }
}

/// Testable core of [`select_transport`] that takes the config explicitly.
pub fn select_transport_with(cfg: &HarnessesConfig, bundle: Option<&str>) -> Result<TransportImpl> {
    Ok(match transport_kind_for(cfg, bundle)? {
        TransportKind::Acp => TransportImpl::Acp(AcpTransport),
        TransportKind::Pty => TransportImpl::Pty(PtyTransport),
    })
}

/// Resolve a bundle name to the [`TransportKind`] that will host it. `None`/empty
/// bundle => `Pty`. Fails loud if the bundle is neither in `harnesses.json` nor a
/// built-in harness slug (mirrors `harness::resolve`).
pub fn transport_kind_for(cfg: &HarnessesConfig, bundle: Option<&str>) -> Result<TransportKind> {
    let Some(bundle) = bundle.filter(|b| !b.is_empty()) else {
        return Ok(TransportKind::Pty);
    };
    Ok(match harness::bundle_transport_with(cfg, bundle)? {
        Transport::Acp | Transport::AppServer => TransportKind::Acp,
        Transport::Pty | Transport::HeadlessExec => TransportKind::Pty,
    })
}

#[cfg(test)]
#[path = "transport/tests.rs"]
mod transport_tests;
