//! Transport-agnostic session hosting seam.
//!
//! A hosted session is opened and driven through one [`SessionTransport`].
//! Transport-specific launch, resume, delivery, liveness, and teardown stay in
//! the implementing module; callers never branch on transport kind.

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

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pty" => Some(Self::Pty),
            "acp" => Some(Self::Acp),
            _ => None,
        }
    }
}

/// Fully-resolved, transport-agnostic launch intent.
#[derive(Clone)]
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
    pub session_name: Option<String>,
    /// Resolved argv incl. base_argv + profile + user flags + agent-def args.
    pub base_command: Vec<String>,
    /// Authoritative session identity allocated before the child starts.
    pub pubkey: String,
    /// Matching signer exposed only to the assigned harness process.
    pub agent_nsec: String,
    pub pty: PtyLaunchSpec,
}

/// PTY-only launch details. Other transports ignore the empty/default value.
#[derive(Clone, Debug, Default)]
pub struct PtyLaunchSpec {
    pub id: Option<String>,
    pub env: Vec<(String, String)>,
    pub env_remove: Vec<String>,
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

/// Complete hosted-session contract. Every selectable transport implements it.
#[async_trait::async_trait]
pub trait SessionTransport: Send + Sync {
    fn kind(&self) -> TransportKind;

    /// Open a brand-new harness session.
    async fn launch(&self, spec: &LaunchSpec) -> Result<SessionEndpoint>;

    /// Reopen a prior session by its native token.
    async fn resume(&self, spec: &LaunchSpec, resume: &ResumeSpec) -> Result<SessionEndpoint>;

    /// Deliver text; `submit` completes the turn.
    async fn deliver(&self, ep: &EndpointRef, text: &str, submit: bool) -> Result<()>;

    fn is_live(&self, ep: &EndpointRef) -> bool;

    /// Delay before the opening prompt can be delivered to a newly launched endpoint.
    fn opening_delivery_delay(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }

    async fn kill(&self, ep: &EndpointRef) -> Result<()>;
}

pub use acp::AcpTransport;
pub use pty::PtyTransport;

/// Type-erased transport selected from a configured harness bundle.
pub struct TransportImpl(Box<dyn SessionTransport>);

impl TransportImpl {
    fn new(transport: impl SessionTransport + 'static) -> Self {
        Self(Box::new(transport))
    }

    pub fn kind(&self) -> TransportKind {
        self.0.kind()
    }

    pub async fn launch(&self, spec: &LaunchSpec) -> Result<SessionEndpoint> {
        self.0.launch(spec).await
    }

    pub async fn resume(&self, spec: &LaunchSpec, resume: &ResumeSpec) -> Result<SessionEndpoint> {
        self.0.resume(spec, resume).await
    }

    pub async fn deliver(&self, ep: &EndpointRef, text: &str, submit: bool) -> Result<()> {
        self.0.deliver(ep, text, submit).await
    }

    pub fn is_live(&self, ep: &EndpointRef) -> bool {
        self.0.is_live(ep)
    }

    pub fn opening_delivery_delay(&self) -> std::time::Duration {
        self.0.opening_delivery_delay()
    }

    pub async fn kill(&self, ep: &EndpointRef) -> Result<()> {
        self.0.kill(ep).await
    }
}

pub fn transport_for_kind(kind: TransportKind) -> TransportImpl {
    match kind {
        TransportKind::Pty => TransportImpl::new(PtyTransport),
        TransportKind::Acp => TransportImpl::new(AcpTransport),
    }
}

/// Pick the exact transport for a required configured bundle.
pub fn select_transport(bundle: &str) -> Result<TransportImpl> {
    let cfg = HarnessesConfig::load()?;
    select_transport_with(&cfg, bundle)
}

/// Map a fully-resolved raw [`Transport`] to its hosting implementation.
fn transport_impl_for(transport: Transport) -> Result<TransportImpl> {
    Ok(match transport {
        Transport::Acp | Transport::AppServer => TransportImpl::new(AcpTransport),
        Transport::Pty => TransportImpl::new(PtyTransport),
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
    })
}

#[cfg(test)]
#[path = "transport/tests.rs"]
mod transport_tests;
