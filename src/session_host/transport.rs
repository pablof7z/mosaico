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
    /// The agent's configured harness bundle name (a `harnesses.json` key).
    /// Distinct from [`Self::slug`]: an agent
    /// `reviewer` may run bundle `codex-acp`. The ACP transport MUST resolve its
    /// harness/driver from this bundle, never from the agent slug (defect #1).
    pub bundle: String,
    /// Optional harness-native named profile from the agent definition.
    pub profile: Option<String>,
    /// Harness-owned native agent definition discovered by Mosaico.
    pub native_agent: Option<crate::agent_catalog::NativeAgentActivation>,
    pub root: String,
    pub abs_path: String,
    pub group: Option<String>,
    pub ephemeral: bool,
    /// Resolved argv incl. base_argv + profile + user flags + agent-def args.
    pub base_command: Vec<String>,
    /// Authoritative session identity allocated before the child starts.
    pub pubkey: String,
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

/// Pick the exact transport for a required configured bundle.
pub fn select_transport(bundle: &str) -> Result<TransportImpl> {
    let cfg = HarnessesConfig::load()?;
    select_transport_with(&cfg, bundle)
}

/// Resolve an agent's configured hosted-session transport without fallback.
pub fn transport_kind_for_slug(slug: &str) -> Result<TransportKind> {
    let launch = crate::identity::agent_launch_config(&crate::config::mosaico_home(), slug)?;
    let cfg = HarnessesConfig::load()?;
    transport_kind_for(&cfg, &launch.harness)
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

pub fn select_transport_with(cfg: &HarnessesConfig, bundle: &str) -> Result<TransportImpl> {
    transport_impl_for(harness::bundle_transport_with(cfg, bundle)?)
}

/// Resolve a required bundle to the hosted-session transport kind.
pub fn transport_kind_for(cfg: &HarnessesConfig, bundle: &str) -> Result<TransportKind> {
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
