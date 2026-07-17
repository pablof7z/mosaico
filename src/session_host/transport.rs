//! Transport-agnostic session hosting seam.
//!
//! A hosted session is opened and driven through one [`SessionTransport`].
//! Transport-specific launch, resume, delivery, liveness, and teardown stay in
//! the implementing module; callers never branch on transport kind.

pub mod acp;
mod acp_runtime;
mod acp_spawn;
pub mod pty;
mod types;

use anyhow::Result;

use crate::harness::{self, config::HarnessesConfig, Transport};

/// Which transport hosts a session. This is the persisted runtime contract;
/// configured app-server sessions never collapse into ACP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransportKind {
    Pty,
    Acp,
    AppServer,
}

impl TransportKind {
    pub const ALL: [Self; 3] = [Self::Pty, Self::Acp, Self::AppServer];

    pub fn as_str(&self) -> &'static str {
        match self {
            TransportKind::Pty => "pty",
            TransportKind::Acp => "acp",
            TransportKind::AppServer => "app-server",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pty" => Some(Self::Pty),
            "acp" => Some(Self::Acp),
            "app-server" => Some(Self::AppServer),
            _ => None,
        }
    }

    pub fn locator_kind(self) -> &'static str {
        match self {
            TransportKind::Pty => crate::state::LOCATOR_PTY,
            TransportKind::Acp => crate::state::LOCATOR_ACP,
            TransportKind::AppServer => crate::state::LOCATOR_APP_SERVER,
        }
    }

    pub fn from_locator_kind(locator_kind: &str) -> Option<Self> {
        match locator_kind {
            crate::state::LOCATOR_PTY => Some(TransportKind::Pty),
            crate::state::LOCATOR_ACP => Some(TransportKind::Acp),
            crate::state::LOCATOR_APP_SERVER => Some(TransportKind::AppServer),
            _ => None,
        }
    }
}

pub use types::*;

/// Complete hosted-session contract. Every selectable transport implements it.
#[async_trait::async_trait]
pub trait SessionTransport: Send + Sync {
    fn kind(&self) -> TransportKind;

    /// Prepare transport-owned launch inputs after harness resolution.
    fn prepare_launch(
        &self,
        _resolved: &mut crate::harness::ResolvedHarness,
        _endpoint_id: String,
    ) -> Result<PreparedLaunch> {
        Ok(PreparedLaunch::default())
    }

    /// Open a brand-new harness session.
    async fn launch(&self, spec: &LaunchSpec) -> Result<SessionEndpoint>;

    /// Reopen a prior session by its native token.
    async fn resume(&self, spec: &LaunchSpec, resume: &ResumeSpec) -> Result<SessionEndpoint>;

    /// Deliver text; `submit` completes the turn.
    async fn deliver(&self, ep: &EndpointRef, text: &str, submit: bool) -> Result<()>;

    fn is_live(&self, ep: &EndpointRef) -> bool;

    /// Whether ordinary endpoint output currently has a visible presentation.
    fn output_is_visible(&self, _ep: &EndpointRef) -> bool {
        false
    }

    /// Transport-owned operator projection. Non-PTY transports are not attachable.
    fn describe(&self, ep: &EndpointRef) -> EndpointDescriptor {
        EndpointDescriptor {
            id: ep.endpoint_id.clone(),
            kind: self.kind(),
            live: self.is_live(ep),
            attachable: false,
            cwd: None,
            command: Vec::new(),
        }
    }

    /// Delay before the opening prompt can be delivered to a newly launched endpoint.
    fn opening_delivery_delay(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }

    async fn kill(&self, ep: &EndpointRef) -> Result<()>;
}

pub use acp::RpcTransport;
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

    pub fn prepare_launch(
        &self,
        resolved: &mut crate::harness::ResolvedHarness,
        endpoint_id: String,
    ) -> Result<PreparedLaunch> {
        self.0.prepare_launch(resolved, endpoint_id)
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

    pub fn output_is_visible(&self, ep: &EndpointRef) -> bool {
        self.0.output_is_visible(ep)
    }

    pub fn describe(&self, ep: &EndpointRef) -> EndpointDescriptor {
        self.0.describe(ep)
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
        TransportKind::Acp | TransportKind::AppServer => {
            TransportImpl::new(RpcTransport::new(kind))
        }
    }
}

/// Resolve a persisted hosted-session locator through the sole transport table.
pub fn transport_for_locator(
    locator: &crate::state::SessionLocator,
) -> Option<(TransportImpl, EndpointRef)> {
    let kind = TransportKind::from_locator_kind(&locator.locator_kind)?;
    Some((
        transport_for_kind(kind),
        EndpointRef {
            kind,
            endpoint_id: locator.locator_value.clone(),
        },
    ))
}

/// The hosted endpoint bound to a session, if one exists.
pub fn hosted_endpoint_for(
    store: &crate::state::Store,
    session: &crate::state::Session,
) -> Result<HostedEndpoint> {
    let Some(kind) = TransportKind::parse(&session.admitted_transport) else {
        return Ok(HostedEndpoint::Unhosted);
    };
    let Some(locator) = store.locator_for_session(
        &session.pubkey,
        &session.observed_harness,
        kind.locator_kind(),
    )?
    else {
        return Ok(HostedEndpoint::Unavailable { kind });
    };
    Ok(HostedEndpoint::Resolved {
        transport: transport_for_kind(kind),
        endpoint: EndpointRef {
            kind,
            endpoint_id: locator.locator_value,
        },
    })
}

/// Pick the exact transport for a required configured bundle.
pub fn select_transport(bundle: &str) -> Result<TransportImpl> {
    let cfg = HarnessesConfig::load()?;
    select_transport_with(&cfg, bundle)
}

/// Map a fully-resolved raw [`Transport`] to its hosting implementation.
fn transport_impl_for(transport: Transport) -> Result<TransportImpl> {
    Ok(match transport {
        Transport::Acp => TransportImpl::new(RpcTransport::new(TransportKind::Acp)),
        Transport::AppServer => TransportImpl::new(RpcTransport::new(TransportKind::AppServer)),
        Transport::Pty => TransportImpl::new(PtyTransport),
    })
}

pub fn select_transport_with(cfg: &HarnessesConfig, bundle: &str) -> Result<TransportImpl> {
    transport_impl_for(harness::bundle_transport_with(cfg, bundle)?)
}

/// Resolve a required bundle to the hosted-session transport kind.
pub fn transport_kind_for(cfg: &HarnessesConfig, bundle: &str) -> Result<TransportKind> {
    Ok(match harness::bundle_transport_with(cfg, bundle)? {
        Transport::Acp => TransportKind::Acp,
        Transport::AppServer => TransportKind::AppServer,
        Transport::Pty => TransportKind::Pty,
    })
}

#[cfg(test)]
#[path = "transport/tests.rs"]
mod transport_tests;
