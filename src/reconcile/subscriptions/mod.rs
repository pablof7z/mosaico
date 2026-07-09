//! Refcounted, per-entity relay-subscription reconciler.
//! This is the honest-Trellis replacement for the retired aggregate
//! `SubscriptionRegistry`. Each covered entity — a channel `#h`, a group-state
//! `#d`, an addressed pubkey `#p` — is its OWN [`ResourceKey`] with its OWN
//! narrow REQ (see [`crate::fabric::subscriptions`]). An entity's REQ is opened
//! ONCE and closed when the LAST owner stops needing it; it is never mutated, so
//! the relay never replays a shrunk aggregate. That kills the unbounded-leak bug
//! AND makes teardown correct.

mod keys;
mod preview;
pub(crate) mod probe;
pub(crate) mod replay;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use nostr_sdk::prelude::{Filter, SubscriptionId};
use serde::{Deserialize, Serialize};
use trellis_core::{
    AuditExplanationLevel, DependencyList, Graph, GraphResult, InputNode, ResourceCommand,
    ResourceCommandExplanation, ResourceCommandKind, ResourceKey, ScopeId, TransactionOptions,
    TransactionResult,
};

use crate::reconcile::labels::NodeLabels;
pub(crate) use keys::SubCommand;
use keys::{id_from_key, plan_subs, sub_key, Space, SubKey};

/// Host effect the daemon applies via the transport.
#[derive(Clone, Debug, PartialEq)]
pub enum SubEffect {
    Open { id: SubscriptionId, filter: Filter },
    Close { id: SubscriptionId },
    Replace { id: SubscriptionId, filter: Filter },
}

/// The daemon's current coverage, split by owner so channels refcount per scope.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageSnapshot {
    /// Explicitly subscribed roots + channels any local/ordinal pubkey manages
    /// or is a member of. Owned by the daemon scope.
    pub daemon_channels: BTreeSet<String>,
    /// Addressed pubkeys: local durable + ordinal + live transient session keys +
    /// backend identity. Owned by the daemon scope.
    pub addressed_pubkeys: BTreeSet<String>,
    /// Archived channels are excluded from all `#h`/`#d` coverage.
    pub archived_channels: BTreeSet<String>,
    /// Alive sessions and the channels each has joined. Each session is its own
    /// scope, so its channels refcount independently.
    pub sessions: BTreeMap<String, BTreeSet<String>>,
}

/// Handles for one alive session's scope + its joined-channel input.
#[derive(Clone, Copy)]
struct SessionNodes {
    scope: ScopeId,
    channels: InputNode<BTreeSet<String>>,
}

/// Refcounted per-entity subscription reconciler over a `Graph<SubCommand>`.
#[derive(Clone)]
pub struct SubscriptionReconciler {
    graph: Graph<SubCommand>,
    daemon_scope: ScopeId,
    daemon_channels: InputNode<BTreeSet<String>>,
    addressed_pubkeys: InputNode<BTreeSet<String>>,
    archived_channels: InputNode<BTreeSet<String>>,
    sessions: BTreeMap<String, SessionNodes>,
    labels: NodeLabels,
}

impl SubscriptionReconciler {
    /// Build the daemon scope: durable-channel + pubkey inputs, the archived
    /// filter, the derived non-archived channel set, the `(space, entity)`
    /// collection, and the planner that opens/closes each entity's REQ.
    pub fn new() -> GraphResult<Self> {
        let mut graph = Graph::<SubCommand>::new_with_command_type();
        let mut labels = NodeLabels::new();
        let mut tx = graph.begin_transaction()?;

        let daemon_scope = tx.create_scope("daemon-subs")?;

        let daemon_channels = tx.input::<BTreeSet<String>>("daemon-channels")?;
        labels.record(daemon_channels.id(), "subscriptions/daemon/channels");
        tx.set_input(daemon_channels, BTreeSet::new())?;
        let addressed_pubkeys = tx.input::<BTreeSet<String>>("addressed-pubkeys")?;
        labels.record(
            addressed_pubkeys.id(),
            "subscriptions/daemon/addressed_pubkeys",
        );
        tx.set_input(addressed_pubkeys, BTreeSet::new())?;
        let archived_channels = tx.input::<BTreeSet<String>>("archived-channels")?;
        labels.record(
            archived_channels.id(),
            "subscriptions/daemon/archived_channels",
        );
        tx.set_input(archived_channels, BTreeSet::new())?;

        // Derived: the daemon's live channels are its candidates minus archived.
        let live_channels = tx.derived(
            "daemon-live-channels",
            DependencyList::new([daemon_channels.id(), archived_channels.id()])?,
            move |ctx| {
                let archived = ctx.input(archived_channels)?;
                Ok(ctx
                    .input(daemon_channels)?
                    .difference(archived)
                    .cloned()
                    .collect::<BTreeSet<String>>())
            },
        )?;

        // Collection: one entity per (space, id) — `#h` + `#d` per channel and
        // `#p` per addressed pubkey.
        let daemon_subs = tx.set_collection::<SubKey>(
            "daemon-subs",
            DependencyList::new([live_channels.id(), addressed_pubkeys.id()])?,
            move |ctx| {
                let mut out = BTreeSet::new();
                for ch in ctx.derived(live_channels)? {
                    out.insert((Space::ChannelH, ch.clone()));
                    out.insert((Space::GroupStateD, ch.clone()));
                }
                for pk in ctx.input(addressed_pubkeys)? {
                    out.insert((Space::PubkeyP, pk.clone()));
                }
                Ok(out)
            },
        )?;
        labels.record(live_channels.id(), "subscriptions/daemon/live_channels");
        labels.record(daemon_subs.id(), "subscriptions/daemon/subs");
        tx.set_resource_planner(daemon_subs, daemon_scope, plan_subs)?;

        tx.commit()?;
        drop(tx);

        Ok(Self {
            graph,
            daemon_scope,
            daemon_channels,
            addressed_pubkeys,
            archived_channels,
            sessions: BTreeMap::new(),
            labels,
        })
    }

    pub fn labels(&self) -> &NodeLabels {
        &self.labels
    }

    pub fn graph_node_count(&self) -> usize {
        self.graph.nodes().count()
    }

    /// Full recompute from the current canonical coverage. Sets the daemon inputs,
    /// creates a scope+input+collection+planner for any newly-alive session,
    /// updates each live session's joined channels, and CLOSES the scope of any
    /// session no longer present (tearing down its solely-owned REQs). Returns the
    /// resulting Open/Close/Replace effects plus the raw receipt.
    pub fn sync(
        &mut self,
        snapshot: &CoverageSnapshot,
    ) -> GraphResult<(Vec<SubEffect>, TransactionResult<SubCommand>)> {
        let mut tx = self.graph.begin_transaction_with_options(opts())?;
        tx.set_input(self.daemon_channels, snapshot.daemon_channels.clone())?;
        tx.set_input(self.addressed_pubkeys, snapshot.addressed_pubkeys.clone())?;
        tx.set_input(self.archived_channels, snapshot.archived_channels.clone())?;

        // Tear down scopes for sessions that are no longer alive.
        let departed: Vec<String> = self
            .sessions
            .keys()
            .filter(|id| !snapshot.sessions.contains_key(*id))
            .cloned()
            .collect();
        for id in departed {
            if let Some(nodes) = self.sessions.remove(&id) {
                tx.close_scope(nodes.scope)?;
            }
        }

        // Upsert every alive session's joined-channel coverage (archived excluded).
        for (id, channels) in &snapshot.sessions {
            let live: BTreeSet<String> = channels
                .difference(&snapshot.archived_channels)
                .cloned()
                .collect();
            if let Some(nodes) = self.sessions.get(id) {
                tx.set_input(nodes.channels, live)?;
            } else {
                let scope = tx.create_scope(format!("session-{id}"))?;
                let channels_input =
                    tx.input::<BTreeSet<String>>(format!("session-{id}-channels"))?;
                self.labels.record(
                    channels_input.id(),
                    format!("subscriptions/session/{id}/channels"),
                );
                tx.set_input(channels_input, live)?;
                let coll = tx.set_collection::<SubKey>(
                    format!("session-{id}-subs"),
                    DependencyList::new([channels_input.id()])?,
                    move |ctx| {
                        let mut out = BTreeSet::new();
                        for ch in ctx.input(channels_input)? {
                            out.insert((Space::ChannelH, ch.clone()));
                            out.insert((Space::GroupStateD, ch.clone()));
                        }
                        Ok(out)
                    },
                )?;
                self.labels
                    .record(coll.id(), format!("subscriptions/session/{id}/subs"));
                tx.set_resource_planner(coll, scope, plan_subs)?;
                self.sessions.insert(
                    id.clone(),
                    SessionNodes {
                        scope,
                        channels: channels_input,
                    },
                );
            }
        }

        let result = tx.commit()?;
        drop(tx);
        let effects = to_effects(&result);
        Ok((effects, result))
    }

    /// Whether channel `h`'s `#h` REQ is currently open (any owner). Drives the
    /// spawn-on-mention replay decision: a channel already covered before a
    /// session became alive may have buffered a mention the live path never
    /// delivered. The latest emitted command for the key reflects live state —
    /// coalesced opens and non-final closes emit nothing, so a non-`Close` last
    /// command means the REQ is open.
    pub fn covers_channel(&self, h: &str) -> bool {
        self.graph
            .why_resource_command(&sub_key(Space::ChannelH, h))
            .map(|e| e.kind != ResourceCommandKind::Close)
            .unwrap_or(false)
    }

    /// The daemon scope id (durable, non-session coverage).
    pub fn daemon_scope(&self) -> ScopeId {
        self.daemon_scope
    }

    /// The number of scopes currently owning a resource key — the authoritative
    /// refcount. A REQ is live iff this is non-zero, and closes exactly when it
    /// falls to zero. (The graph tracks this precisely even for owner releases the
    /// host-facing effect stream coalesces away.)
    pub fn owner_count(&self, key: &ResourceKey) -> usize {
        self.graph.resource_owners(key).map_or(0, BTreeSet::len)
    }

    /// Audit query: why the latest command for a resource key was emitted.
    pub fn why_command(&self, key: &ResourceKey) -> Option<&ResourceCommandExplanation> {
        self.graph.why_resource_command(key)
    }

    /// The full-recompute oracle: incremental state must equal a rebuild.
    pub fn assert_oracle(&self) -> GraphResult<()> {
        self.graph.assert_incremental_equals_full()?;
        Ok(())
    }
}

fn opts() -> TransactionOptions {
    TransactionOptions::default().with_audit_explanations(AuditExplanationLevel::DependencyPaths)
}

/// Translate the graph's resource plan into host effects the daemon applies.
fn to_effects(result: &TransactionResult<SubCommand>) -> Vec<SubEffect> {
    result
        .resource_plan
        .commands()
        .iter()
        .filter_map(|c| match c {
            ResourceCommand::Open { command, .. } => Some(SubEffect::Open {
                id: command.id.clone(),
                filter: command.filter.clone(),
            }),
            ResourceCommand::Replace { command, .. } | ResourceCommand::Refresh { command, .. } => {
                Some(SubEffect::Replace {
                    id: command.id.clone(),
                    filter: command.filter.clone(),
                })
            }
            ResourceCommand::Close { key, .. } => {
                id_from_key(key).map(|id| SubEffect::Close { id })
            }
        })
        .collect()
}
