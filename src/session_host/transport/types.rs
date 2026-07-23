use super::{TransportImpl, TransportKind};

/// How the owner can observe the completion of a submitted turn.
///
/// PTY turns are projected by native harness hooks. RPC turns are daemon-owned,
/// so delivery returns the exact request-completion signal that must close the
/// durable Working -> Idle lifecycle edge.
pub enum DeliveryCompletion {
    /// The harness owns its own hook boundary (PTY).
    ExternallyObserved,
    /// The transport started a fresh daemon-owned turn.
    Managed(tokio::sync::oneshot::Receiver<anyhow::Result<()>>),
    /// An already-working app-server turn accepted a steer. This is not a new
    /// lifecycle edge, but it is the transport's exact work-start equivalent
    /// for a newly delivered message.
    ManagedSteer(tokio::sync::oneshot::Receiver<anyhow::Result<()>>),
}

/// Fully-resolved, transport-agnostic launch intent.
#[derive(Clone)]
pub struct LaunchSpec {
    pub slug: String,
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
    pub prepared: PreparedLaunch,
}

/// PTY-only launch details. Other transports ignore the empty/default value.
#[derive(Clone, Debug, Default)]
pub struct PtyLaunchSpec {
    pub id: Option<String>,
    pub env: Vec<(String, String)>,
    pub env_remove: Vec<String>,
}

/// Immutable runtime inputs captured by the single admission-time resolution.
#[derive(Clone, Debug, Default)]
pub struct PreparedLaunch {
    pub pty: PtyLaunchSpec,
    pub rpc: Option<RpcLaunchSpec>,
}

#[derive(Clone, Debug)]
pub struct RpcLaunchSpec {
    pub driver: &'static crate::harness::HarnessDriver,
    pub argv: Vec<String>,
    pub extra_env: Vec<(String, String)>,
    pub harness: crate::session::Harness,
}

/// A prior session's native resume token.
#[derive(Debug, Clone)]
pub struct ResumeSpec {
    pub native_id: String,
}

/// What the daemon needs after a session opens. The typed kind must survive
/// registration; callers never infer it from transport-specific metadata.
#[derive(Debug)]
pub struct SessionEndpoint {
    pub kind: TransportKind,
    pub endpoint_id: String,
    pub watch_pid: Option<i32>,
    /// The harness-native resume token this session opened with, when the
    /// transport owns it directly (ACP `sessionId` / app-server thread id).
    /// `None` for transports that rely on the harness's own mosaico hook to
    /// report a resume token (PTY). Recorded as the `native_resume` locator at
    /// registration so an online hosted session is resumable without a hook.
    pub native_id: Option<String>,
    pub meta: crate::pty::LaunchMetadata,
}

impl SessionEndpoint {
    pub fn endpoint_ref(&self) -> EndpointRef {
        EndpointRef {
            kind: self.kind,
            endpoint_id: self.endpoint_id.clone(),
        }
    }
}

/// A live-session address the daemon holds after registration.
#[derive(Debug, Clone)]
pub struct EndpointRef {
    pub kind: TransportKind,
    pub endpoint_id: String,
}

/// Resolution of the runtime endpoint admitted on a session record.
pub enum HostedEndpoint {
    /// A native process with no daemon-hosted transport.
    Unhosted,
    /// The session was admitted as hosted, but its exact locator is unavailable.
    Unavailable { kind: TransportKind },
    /// The admitted transport and exact harness-scoped locator both resolved.
    Resolved {
        transport: TransportImpl,
        endpoint: EndpointRef,
    },
}

/// Operator-facing endpoint capabilities projected by the owning transport.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EndpointDescriptor {
    pub id: String,
    pub kind: TransportKind,
    pub live: bool,
    pub attachable: bool,
    pub cwd: Option<String>,
    pub command: Vec<String>,
}
