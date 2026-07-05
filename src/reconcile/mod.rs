//! Trellis reconciliation spine for the tenex-edge daemon.
//!
//! Boundary principle: **Trellis owns decisions; the host owns observations
//! and effects.** Canonical world-facts enter a [`Reconciler`]'s graph as
//! inputs (see [`journal`]); the graph derives sessions/status/outbox and other
//! reconciled state and returns resource *plans* and output *frames* as plain
//! data. The host applies those plans — the graph performs no I/O and never
//! invents a fact the world did not hand it.
//!
//! Bulky payloads (transcripts, raw event bodies) NEVER enter the graph. Only
//! stable pointers/hashes/summaries do: an [`InputFact`] carries a
//! transcript-window *hash* and a distilled title/activity, never the text.
//!
//! This module is the additive foundation for Trellis adoption. It proves the
//! pattern compiles against the real `trellis-core` API and that the
//! full-recompute oracle and audit queries are usable from tenex-edge. The
//! surface reconcilers (real sessions/status/who/outbox planners) land on top
//! of this spine later; nothing here changes existing daemon behavior yet.

pub mod frontier;
pub mod graph;
pub mod hook_context;
pub mod journal;
pub mod labels;
pub mod status;
pub mod subscriptions;

pub use graph::{ReconcileCommand, Reconciler};
pub use hook_context::{
    FrameKind, HookContextOutcome, HookContextReceipt, HookContextReconciler, Shape,
};
pub use journal::InputFact;
pub use labels::{CommitFacts, NodeLabels};
pub use status::{PublishReason, StatusCommand, StatusEffect, StatusOutcome, StatusReconciler};
pub use subscriptions::{CoverageSnapshot, SubEffect, SubscriptionReconciler};
