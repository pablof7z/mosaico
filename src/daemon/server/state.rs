use super::{HostedAgent, PeerTracked, SessionHandle, StatusTailKey, StatusTailSnapshot};
use crate::daemon::tail_event::TailEvent;
use crate::reconcile::{StatusReconciler, SubscriptionReconciler};
use crate::session::Harness;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// Serializes durable agent-record mutations owned by this daemon instance.
pub(super) struct AgentConfigState {
    mutation: Mutex<()>,
}

impl AgentConfigState {
    pub(super) fn new() -> Self {
        Self {
            mutation: Mutex::new(()),
        }
    }

    pub(super) fn mutate<R>(&self, operation: impl FnOnce() -> R) -> R {
        let _guard = self.mutation.lock().expect("agent config mutex poisoned");
        operation()
    }
}

/// Native-agent and harness discovery owned by the catalog monitor.
pub(super) struct CatalogState {
    pub(super) agents: Mutex<crate::agent_catalog::AgentCatalog>,
    pub(super) harnesses: Mutex<Vec<Harness>>,
}

impl CatalogState {
    pub(super) fn new() -> Self {
        Self {
            agents: Mutex::new(crate::agent_catalog::AgentCatalog::default()),
            harnesses: Mutex::new(Vec::new()),
        }
    }
}

/// In-process engines and identities for admitted local sessions.
pub(super) struct SessionRuntimeState {
    pub(super) hosted: Mutex<HashMap<String, HostedAgent>>,
    pub(super) engines: Mutex<HashMap<String, SessionHandle>>,
    pub(super) hook_contexts: crate::turn_context::HookContextStates,
}

impl SessionRuntimeState {
    pub(super) fn new() -> Self {
        Self {
            hosted: Mutex::new(HashMap::new()),
            engines: Mutex::new(HashMap::new()),
            hook_contexts: Mutex::new(HashMap::new()),
        }
    }
}

/// Subscription coverage policy and its serialized apply gate.
pub(super) struct SubscriptionState {
    pub(super) roots: Mutex<Vec<String>>,
    pub(super) reconciler: Mutex<SubscriptionReconciler>,
    pub(super) sync: tokio::sync::Mutex<()>,
}

impl SubscriptionState {
    pub(super) fn new() -> Self {
        Self {
            roots: Mutex::new(Vec::new()),
            reconciler: Mutex::new(SubscriptionReconciler::new()),
            sync: tokio::sync::Mutex::new(()),
        }
    }
}

/// Stateful reconcilers whose policy outlives any one RPC.
pub(super) struct ReconcilerState {
    pub(super) status: Arc<Mutex<StatusReconciler>>,
}

impl ReconcilerState {
    pub(super) fn new(status: StatusReconciler) -> Self {
        Self {
            status: Arc::new(Mutex::new(status)),
        }
    }
}

/// RPC connection lifetime, tail fanout, and daemon shutdown signaling.
pub(super) struct ConnectionState {
    pub(super) tail_tx: tokio::sync::broadcast::Sender<TailEvent>,
    pub(super) open_clients: Mutex<u64>,
    pub(super) shutdown: Notify,
}

impl ConnectionState {
    pub(super) fn new() -> Self {
        Self {
            tail_tx: tokio::sync::broadcast::channel(512).0,
            open_clients: Mutex::new(0),
            shutdown: Notify::new(),
        }
    }
}

/// Bounded, rebuildable relay-facing observations used only for projection and dedup.
pub(super) struct DedupState {
    pub(super) peer_sessions: Mutex<HashMap<(String, String), PeerTracked>>,
    pub(super) events: Mutex<(HashSet<String>, VecDeque<String>)>,
    pub(super) profiles: Mutex<HashSet<String>>,
    pub(super) warming_profiles: Mutex<HashSet<String>>,
    pub(super) last_status: Mutex<HashMap<StatusTailKey, StatusTailSnapshot>>,
}

impl DedupState {
    pub(super) fn new() -> Self {
        Self {
            peer_sessions: Mutex::new(HashMap::new()),
            events: Mutex::new((HashSet::new(), VecDeque::new())),
            profiles: Mutex::new(HashSet::new()),
            warming_profiles: Mutex::new(HashSet::new()),
            last_status: Mutex::new(HashMap::new()),
        }
    }
}
