//! The status reconciler drive seam, extracted from the runtime engine.
//!
//! [`drive`] runs one status-reconciler method under the shared lock (never held
//! across `.await`), signs + enqueues the emitted publish/expire effects onto the
//! durable outbox, and — Slice 8 — records a flattened receipt of the commit
//! keyed by the published kind:30315 event id. When the drive was a distill
//! completion, the caller passes the distill's `window_hash`, threaded into the
//! receipt so `explain event:<id>` rejoins the exact LLM inputs.

use std::sync::Mutex;

use nostr_sdk::prelude::{JsonUtil, Keys, NostrSigner};
use trellis_core::{ResourceCommand, TransactionResult};

use crate::domain::{DomainEvent, Status};
use crate::fabric::provider::Nip29Provider;
use crate::reconcile::{StatusCommand, StatusEffect, StatusOutcome, StatusReconciler};
use crate::state::receipts::NewReceipt;
use crate::state::Store;
use crate::util::now_secs;

/// Run one reconciler method, apply its effects, and record BOTH its receipt (for
/// an effectful publish) and its all-commit ledger row (for EVERY commit, incl.
/// no-ops) — the single status seam. `trigger` names the drive method (`"tick"`,
/// `"distill"`, …) for the ledger. `window_hash` is `Some` only for a distill-
/// completion drive, carrying the join key onto the published 30315's receipt.
pub(crate) async fn drive(
    status: &Mutex<StatusReconciler>,
    provider: &Nip29Provider,
    keys: &Keys,
    store: &Mutex<Store>,
    trigger: &str,
    window_hash: Option<&str>,
    f: impl FnOnce(&mut StatusReconciler) -> trellis_core::GraphResult<StatusOutcome>,
) {
    // Time the commit at the host boundary (the ledgers never read the clock).
    let start = std::time::Instant::now();
    let (outcome, facts) = {
        let mut rec = status.lock().expect("status reconciler poisoned");
        let outcome = f(&mut rec).ok();
        // Flatten EVERY commit (incl. no-ops) through the surface's labels.
        let facts = outcome.as_ref().map(|o| {
            let mut facts = crate::reconcile::CommitFacts::from_result(
                rec.labels(),
                &o.result,
                rec.graph_node_count(),
            );
            facts.graph_resources = rec.state_rows().len() as i64;
            facts
        });
        (outcome, facts)
    };
    let duration_us = start.elapsed().as_micros() as i64;
    let Some(outcome) = outcome else { return };
    let event_ids = apply_status_effects(outcome.effects, provider, keys, store).await;
    let trigger_ref = status_session_id(&outcome.result);
    record_status_receipt(store, window_hash, &outcome.result, &event_ids);
    if let Some(facts) = facts {
        let g = store.lock().expect("store mutex poisoned");
        crate::instrument::record_commit(
            &g,
            "status",
            trigger,
            trigger_ref,
            &facts,
            duration_us,
            crate::instrument::now_millis(),
        );
    }
}

fn status_session_id(result: &TransactionResult<StatusCommand>) -> Option<&str> {
    result
        .resource_plan
        .commands()
        .iter()
        .find_map(|c| match c {
            ResourceCommand::Open { command, .. }
            | ResourceCommand::Replace { command, .. }
            | ResourceCommand::Refresh { command, .. } => Some(command.session_id.as_str()),
            ResourceCommand::Close { .. } => None,
        })
}

/// Flatten a committed status transaction into a receipt keyed by the published
/// event id. Only commits that actually published (non-empty `event_ids`) are
/// recorded, so a no-op tick leaves no noise. Off the graph path, host-side.
fn record_status_receipt(
    store: &Mutex<Store>,
    window_hash: Option<&str>,
    result: &TransactionResult<StatusCommand>,
    event_ids: &[String],
) {
    let Some(artifact_ref) = event_ids.first().cloned() else {
        return;
    };
    let session_id = status_session_id(result);
    let row = NewReceipt {
        surface: "status".into(),
        transaction_id: result.transaction_id.get() as i64,
        revision: result.revision.get() as i64,
        changed_summary: crate::instrument::changed_summary_json(
            &result.changed_inputs,
            &result.changed_derived_nodes,
            &result.changed_collection_nodes,
            session_id,
            window_hash,
        ),
        commands: crate::instrument::commands_json(result.resource_plan.commands()),
        artifact_ref: Some(artifact_ref),
        created_at: crate::instrument::now_millis(),
    };
    let g = store.lock().expect("store mutex poisoned");
    crate::instrument::record_receipt(&g, row);
}

/// Sign + enqueue every status effect and return the signed event ids in order —
/// the first is the receipt's `artifact_ref`. The reconciler DECIDES; the outbox
/// drainer EXECUTES (publishes the exact signed JSON we enqueue here).
async fn apply_status_effects(
    effects: Vec<StatusEffect>,
    provider: &Nip29Provider,
    keys: &Keys,
    store: &Mutex<Store>,
) -> Vec<String> {
    let mut ids = Vec::new();
    for effect in effects {
        let status = match effect {
            StatusEffect::Publish { status, .. } | StatusEffect::Expire { status } => status,
        };
        if let Some(id) = enqueue_status(provider, keys, store, status, now_secs()).await {
            ids.push(id);
        }
    }
    ids
}

/// Encode + sign the status and park the signed JSON on the `outbox`, returning
/// the signed event id. The drainer publishes it; the engine never talks to the
/// relay for status.
async fn enqueue_status(
    provider: &Nip29Provider,
    keys: &Keys,
    store: &Mutex<Store>,
    status: Status,
    now: u64,
) -> Option<String> {
    let builder = match provider.encode(&DomainEvent::Status(status)) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "enqueue_status: encoding status event failed — skipping this heartbeat");
            return None;
        }
    };
    let unsigned = builder.build(keys.public_key());
    let signed = match keys.sign_event(unsigned).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "enqueue_status: signing status event failed — skipping this heartbeat");
            return None;
        }
    };
    let json = signed.as_json();
    match store.lock() {
        Ok(g) => {
            if let Err(e) = g.enqueue_outbox(&json, now) {
                tracing::error!(error = %e, "enqueue_status: enqueue_outbox failed — status not published this cycle");
                return None;
            }
        }
        Err(_) => {
            tracing::error!(
                "enqueue_status: store mutex poisoned — status not published this cycle"
            );
            return None;
        }
    }
    Some(signed.id.to_hex())
}
