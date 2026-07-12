//! Transport-agnostic session hosting seam.
//!
//! A "hosted session" is opened over exactly one transport. The portable PTY is
//! driven DIRECTLY (`pty::spawn_session` / `pty::inject`, argv shaped by the
//! `registry` sniffers) and reaches [`PtyTransport`] only for `kind`/`kill`.
//! `AcpTransport` is the JSON-RPC backend (ACP / codex app-server) for harnesses
//! that expose an `RpcTurn` model, and is the sole full [`SessionTransport`] impl
//! — its `launch`/`resume`/`deliver`/`is_live` are the ones actually invoked.
//!
//! Object dispatch is via the [`TransportImpl`] enum rather than `dyn` (native
//! async-fn-in-trait is not object-safe and we avoid pulling `async-trait`).
//! The [`SessionTransport`] trait documents the RPC contract `AcpTransport`
//! fulfils.

pub mod acp;
mod acp_runtime;
mod acp_spawn;
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
    /// The agent's configured harness *bundle* name (a `harnesses.json` key or a
    /// built-in harness slug), if any. Distinct from [`Self::slug`]: an agent
    /// `reviewer` may run bundle `codex-acp`. The ACP transport MUST resolve its
    /// harness/driver from this bundle, never from the agent slug (defect #1).
    /// `None` for agents with no configured bundle (PTY agents).
    pub bundle: Option<String>,
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

/// The RPC transport contract, fulfilled by [`AcpTransport`]. Each method maps to
/// an existing daemon behavior over JSON-RPC (ACP / codex app-server). The PTY
/// path does NOT implement this trait — it is driven directly (relaunch-with-flag
/// + bracketed-paste), and [`PtyTransport`] exposes only `kind`/`kill`.
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
///
/// Only `kind` and `kill` are dispatched here: the ACP path invokes
/// `AcpTransport`'s [`SessionTransport`] methods on the concrete variant directly
/// (`launch.rs`/`delivery.rs`), and the PTY path is driven outside the trait
/// entirely. The former per-method enum forwarders for
/// `launch`/`resume`/`deliver`/`is_live` were dead and removed.
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
/// transport is `Acp`/`AppServer` selects [`AcpTransport`]; `Pty` selects
/// [`PtyTransport`].
///
/// Defect #5: a `HeadlessExec` bundle is a **hard error** here, NOT collapsed onto
/// the PTY. Headless bundles run a one-shot argv (`claude -p` / `codex exec` /
/// `opencode run`) and must reach the real exec path (`session_host::exec`); an
/// interactive PTY supervisor would run that argv as a long-lived TTY and never
/// complete a turn. Until headless hosting is wired through this seam, refusing
/// loudly beats a silently-wrong transport. This bail is deliberately NOT swallowed
/// by the fail-open path below — a cleanly-resolved-but-unsupported transport is a
/// real error, unlike a missing/malformed config.
///
/// Defect #3 contract: this launch-time entry point otherwise **fails open to the
/// PTY** (with a loud `WARN`) when `harnesses.json` is missing or malformed, rather
/// than aborting a launch that previously worked. A configured-but-unresolvable
/// bundle is almost always a corrupt/edited config, and a bundle-carrying agent
/// that used to launch on the PTY (its bundle resolving to `Pty`) must not be
/// bricked by an unrelated config error. This mirrors the `agent_harness_bundle`
/// "absent config => `None` => PTY" fail-open. The pure core
/// [`select_transport_with`] / [`transport_kind_for`] stay fail-loud so the strict
/// resolution contract remains unit-testable; only this IO wrapper softens it. The
/// cost — a genuinely-ACP agent silently launching on the PTY under a corrupt
/// config — surfaces loudly downstream (an ACP-only agent has no PTY `commands`
/// entry, so the PTY launch then fails at command resolution).
pub fn select_transport(bundle: Option<&str>) -> Result<TransportImpl> {
    transport_impl_for(resolve_transport_fail_open(bundle))
}

/// Resolve the transport kind for an agent `slug` from its configured harness
/// bundle, failing open to [`TransportKind::Pty`] on any resolution error
/// (mirrors [`select_transport`]). Used by the transport-aware delivery path,
/// which must classify a live session's endpoint as PTY vs. ACP.
///
/// A `HeadlessExec` bundle classifies as `Pty` here (rather than erroring):
/// `select_transport` refuses to *launch* one, so no headless endpoint is ever
/// live, and this classification branch is only a safe default for the
/// already-live delivery/liveness path.
pub fn transport_kind_for_slug(slug: &str) -> TransportKind {
    let bundle = crate::identity::agent_harness_bundle(&crate::config::edge_home(), slug);
    match resolve_transport_fail_open(bundle.as_deref()) {
        Transport::Acp | Transport::AppServer => TransportKind::Acp,
        Transport::Pty | Transport::HeadlessExec => TransportKind::Pty,
    }
}

/// Map a fully-resolved raw [`Transport`] to its hosting [`TransportImpl`], with
/// the defect #5 hard-bail on `HeadlessExec` (see [`select_transport`]).
fn transport_impl_for(transport: Transport) -> Result<TransportImpl> {
    Ok(match transport {
        Transport::Acp | Transport::AppServer => TransportImpl::Acp(AcpTransport),
        Transport::Pty => TransportImpl::Pty(PtyTransport),
        Transport::HeadlessExec => anyhow::bail!(
            "headless-exec harness bundles are not yet wired into session hosting; \
             refusing to launch (a one-shot exec argv must not run inside the \
             interactive PTY supervisor — route it through session_host::exec once wired)"
        ),
    })
}

/// Shared fail-open resolution of a bundle to its raw [`Transport`]: `None`/empty
/// bundle => `Pty` (no config touched); otherwise load `harnesses.json` and
/// resolve, falling back to `Pty` with a `WARN` if the config is missing/malformed
/// or the bundle is unresolvable. NOTE: this does NOT special-case `HeadlessExec` —
/// callers decide (launch hard-bails via [`transport_impl_for`], delivery
/// classification downgrades to PTY).
fn resolve_transport_fail_open(bundle: Option<&str>) -> Transport {
    // Short-circuit the no-bundle case WITHOUT touching harnesses.json: an agent
    // with no configured bundle always launches on the PTY, and must not be made
    // to depend on (or fail because of) a malformed harnesses.json it never uses.
    let Some(bundle) = bundle.filter(|b| !b.is_empty()) else {
        return Transport::Pty;
    };
    resolve_transport_fail_open_with(bundle, HarnessesConfig::load())
}

/// Testable core of [`resolve_transport_fail_open`]: given a non-empty bundle and
/// the (possibly failed) config load, resolve the raw transport, failing open to
/// `Transport::Pty` with a `WARN` on any config/resolution error (defect #3). Kept
/// separate from the IO so the fail-open contract is unit-testable without touching
/// the filesystem/`edge_home`.
fn resolve_transport_fail_open_with(
    bundle: &str,
    cfg: anyhow::Result<HarnessesConfig>,
) -> Transport {
    match cfg.and_then(|cfg| harness::bundle_transport_with(&cfg, bundle)) {
        Ok(transport) => transport,
        Err(e) => {
            tracing::warn!(
                bundle = %bundle,
                error = %format!("{e:#}"),
                "harness bundle transport resolution failed (missing/malformed harnesses.json?); \
                 falling back to PTY transport"
            );
            Transport::Pty
        }
    }
}

/// Testable core of [`select_transport`] that takes the config explicitly. Fails
/// loud (including the defect #5 `HeadlessExec` bail) rather than fail-open.
pub fn select_transport_with(cfg: &HarnessesConfig, bundle: Option<&str>) -> Result<TransportImpl> {
    let transport = match bundle.filter(|b| !b.is_empty()) {
        Some(bundle) => harness::bundle_transport_with(cfg, bundle)?,
        None => Transport::Pty,
    };
    transport_impl_for(transport)
}

/// Resolve a bundle name to the [`TransportKind`] that will host it. `None`/empty
/// bundle => `Pty`. Fails loud if the bundle is neither in `harnesses.json` nor a
/// built-in harness slug (mirrors `harness::resolve`), and — defect #5 — if the
/// bundle resolves to `HeadlessExec`, which has no `TransportKind` (it is not a
/// hosted-session transport; it must reach `session_host::exec`).
pub fn transport_kind_for(cfg: &HarnessesConfig, bundle: Option<&str>) -> Result<TransportKind> {
    let Some(bundle) = bundle.filter(|b| !b.is_empty()) else {
        return Ok(TransportKind::Pty);
    };
    Ok(match harness::bundle_transport_with(cfg, bundle)? {
        Transport::Acp | Transport::AppServer => TransportKind::Acp,
        Transport::Pty => TransportKind::Pty,
        Transport::HeadlessExec => anyhow::bail!(
            "harness bundle {bundle:?} uses the headless-exec transport, which is not a \
             hosted-session transport and must not be collapsed onto the PTY (defect #5)"
        ),
    })
}

#[cfg(test)]
#[path = "transport/tests.rs"]
mod transport_tests;
