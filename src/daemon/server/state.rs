use super::{HostedAgent, PeerTracked, SessionHandle, StatusTailKey, StatusTailSnapshot};
use crate::daemon::tail_event::TailEvent;
use crate::reconcile::{StatusReconciler, SubscriptionReconciler};
use crate::session::Harness;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
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

/// Coalesces backend-profile refresh triggers without retaining signed events.
pub(super) struct ProfileRefreshState {
    dirty: AtomicBool,
    running: AtomicBool,
}

impl ProfileRefreshState {
    pub(super) fn new() -> Self {
        Self {
            dirty: AtomicBool::new(false),
            running: AtomicBool::new(false),
        }
    }

    /// Record a newer desired profile and claim the sole worker when idle.
    pub(super) fn request(&self) -> bool {
        self.dirty.store(true, Ordering::Release);
        self.running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub(super) fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::AcqRel)
    }

    pub(super) fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Acquire)
    }

    /// Release ownership, reclaiming it if a request raced with release.
    pub(super) fn release_or_reclaim(&self) -> bool {
        self.running.store(false, Ordering::Release);
        self.is_dirty()
            && self
                .running
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
    }
}

/// In-process engines and identities for admitted local sessions.
pub(super) struct SessionRuntimeState {
    pub(super) hosted: Mutex<HashMap<String, HostedAgent>>,
    pub(super) engines: Mutex<HashMap<String, SessionHandle>>,
    pub(super) hook_contexts: crate::turn_context::HookContextStates,
    pub(super) channel_nudges: Mutex<crate::channel_nudge::ChannelNudgeState>,
    pub(super) active_waits: Arc<Mutex<super::channel_wait::ActiveWaitRegistry>>,
    pub(super) extension_delivery:
        Arc<Mutex<super::session_delivery::ActiveExtensionDeliveryRegistry>>,
    pub(super) pty_probe_failures: Mutex<HashMap<(String, u64), u8>>,
}

impl SessionRuntimeState {
    pub(super) fn new() -> Self {
        Self {
            hosted: Mutex::new(HashMap::new()),
            engines: Mutex::new(HashMap::new()),
            hook_contexts: Mutex::new(HashMap::new()),
            channel_nudges: Mutex::new(crate::channel_nudge::ChannelNudgeState::default()),
            active_waits: Arc::new(Mutex::new(
                super::channel_wait::ActiveWaitRegistry::default(),
            )),
            extension_delivery: Arc::new(Mutex::new(
                super::session_delivery::ActiveExtensionDeliveryRegistry::default(),
            )),
            pty_probe_failures: Mutex::new(HashMap::new()),
        }
    }
}

/// Subscription coverage policy and its serialized apply gate.
pub(super) struct SubscriptionState {
    pub(super) roots: Mutex<Vec<String>>,
    pub(super) reconciler: Mutex<SubscriptionReconciler>,
    pub(super) sync: tokio::sync::Mutex<()>,
    pub(super) profile_authors: Mutex<std::collections::BTreeSet<String>>,
    /// The one stable NIP-29 group-records demand. Product coverage owns its
    /// lifetime; the runtime only maps the stable id to an SDK subscription.
    pub(super) group_records: Mutex<super::group_records::GroupRecordsWatch>,
}

impl SubscriptionState {
    pub(super) fn new() -> Self {
        Self {
            roots: Mutex::new(Vec::new()),
            reconciler: Mutex::new(SubscriptionReconciler::new()),
            sync: tokio::sync::Mutex::new(()),
            profile_authors: Mutex::new(std::collections::BTreeSet::new()),
            group_records: Mutex::new(super::group_records::GroupRecordsWatch::default()),
        }
    }
}

/// Stateful reconcilers whose policy outlives any one RPC.
pub(super) struct ReconcilerState {
    pub(super) status: Arc<Mutex<StatusReconciler>>,
    pub(super) presence_publisher: crate::presence_publisher::PresencePublisher,
}

impl ReconcilerState {
    pub(super) fn new(
        status: StatusReconciler,
        presence_publisher: crate::presence_publisher::PresencePublisher,
    ) -> Self {
        Self {
            status: Arc::new(Mutex::new(status)),
            presence_publisher,
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

/// Bounded, rebuildable projection state for ephemeral tail events only.
pub(super) struct DedupState {
    pub(super) peer_sessions: Mutex<HashMap<(String, String), PeerTracked>>,
    pub(super) profiles: Mutex<HashSet<String>>,
    pub(super) last_status: Mutex<HashMap<StatusTailKey, StatusTailSnapshot>>,
    /// Wakes the demux loop to recompute its claim-retry timer after a
    /// handler records a failed attempt.
    pub(super) claim_retry_scheduled: tokio::sync::Notify,
    /// The running demux task; replaced (and the old one aborted) on respawn.
    pub(super) demux_task: Mutex<Option<tokio::task::AbortHandle>>,
}

impl DedupState {
    pub(super) fn new() -> Self {
        Self {
            peer_sessions: Mutex::new(HashMap::new()),
            profiles: Mutex::new(HashSet::new()),
            last_status: Mutex::new(HashMap::new()),
            claim_retry_scheduled: tokio::sync::Notify::new(),
            demux_task: Mutex::new(None),
        }
    }
}
