//! Local row vocabulary, replacing `fava::Row` and `fava::RowDelta`.
//!
//! These types have the same field names as their Fava counterparts so that
//! most call sites need only an import-path change.

use nostr::event::Event;
use nostr::event::EventId;

/// One relay-delivered event row.
///
/// Mirrors the public field shape of `fava::Row`.  Sources are omitted because
/// fava tracks provenance inside `QuerySnapshot.evidence` and Mosaico never
/// re-exposes them to product callers.
#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub(crate) event: Event,
}

/// Delta applied to a retained fava observation.
///
/// The `SourcesGrew` variant from `fava::RowDelta` is absent: fava delivers
/// complete replacement snapshots, so source growth is implicit in every
/// snapshot delivery and never needs its own delta.
#[derive(Clone, Debug)]
pub(crate) enum RowDelta {
    Added(Row),
    Removed(EventId),
}
