//! The status reconciler drive seam, extracted from the runtime engine.
//!
//! [`drive`] runs one status-reconciler method under the shared lock, signs +
//! enqueues publish effects, records receipts/commits, and threads distill
//! `window_hash` values so `explain event:<id>` rejoins the exact LLM inputs.

use std::sync::Mutex;

use nostr_sdk::prelude::{JsonUtil, Keys, NostrSigner};
use trellis_core::{ResourceCommand, TransactionResult};

use crate::domain::{DomainEvent, Status};
use crate::fabric::provider::Nip29Provider;
use crate::reconcile::{
    InputFact, StatusCommand, StatusDrive, StatusEffect, StatusOutcome, StatusReconciler,
};
use crate::state::receipts::NewReceipt;
use crate::state::Store;
use crate::util::now_secs;

pub(crate) struct DriveMeta<'a> {
    pub trigger: &'a str,
    pub window_hash: Option<&'a str>,
    pub replay_fact: Option<InputFact>,
}

/// Run one status transaction, apply effects, and record receipt/commit evidence.
pub(crate) async fn drive(
    status: &Mutex<StatusReconciler>,
    provider: &Nip29Provider,
    keys: &Keys,
    store: &Mutex<Store>,
    outbox: &Mutex<crate::reconcile::OutboxReconciler>,
    meta: DriveMeta<'_>,
    f: impl FnOnce(&mut StatusReconciler) -> trellis_core::GraphResult<StatusOutcome>,
) {
    // Time the commit at the host boundary (the ledgers never read the clock).
    let start = std::time::Instant::now();
    let (outcome, facts, replay_seed, preview) = {
        let mut rec = status.lock().expect("status reconciler poisoned");
        let replay_seed = meta
            .replay_fact
            .as_ref()
            .and_then(status_replay_seed_session_id)
            .and_then(|id| rec.replay_seed(id));
        let preview = meta.replay_fact.as_ref().and_then(|fact| {
            rec.preview_fact(fact)
                .map_err(|e| {
                    tracing::error!(error = ?e, "status preview failed before commit");
                    e
                })
                .ok()
                .flatten()
        });
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
        (outcome, facts, replay_seed, preview.map(|p| p.result))
    };
    let duration_us = start.elapsed().as_micros() as i64;
    let Some(outcome) = outcome else { return };
    let effects = outcome.effects;
    if !effects.is_empty() && !preview_matches(preview.as_ref(), &outcome.result) {
        tracing::error!(
            trigger = meta.trigger,
            "status effects blocked: committed plan was not previewed first"
        );
        return;
    }
    let event_ids = apply_status_effects(
        effects,
        provider,
        keys,
        store,
        outbox,
        &outcome.result,
        preview
            .as_ref()
            .expect("effectful status commit has preview"),
    )
    .await;
    let trigger_ref = status_session_id(&outcome.result);
    record_status_receipt(store, meta.window_hash, &outcome.result, &event_ids);
    if let Some(facts) = facts {
        let created_at = crate::instrument::now_millis();
        let g = store.lock().expect("store mutex poisoned");
        crate::instrument::record_commit(
            &g,
            "status",
            meta.trigger,
            trigger_ref,
            &facts,
            duration_us,
            created_at,
        );
        if let Some(fact) = meta.replay_fact {
            let mut replay_facts = Vec::new();
            if let Some(seed) = replay_seed {
                replay_facts.push(InputFact::StatusDrive(StatusDrive::SessionStarted(seed)));
            }
            replay_facts.push(fact);
            crate::replay_capsules::record_many(
                &g,
                "status",
                meta.trigger,
                trigger_ref,
                replay_facts,
                created_at,
            );
        }
    }
}

fn status_replay_seed_session_id(fact: &InputFact) -> Option<&str> {
    match fact {
        InputFact::StatusDrive(StatusDrive::SessionStarted(_)) => None,
        InputFact::StatusDrive(StatusDrive::TurnStarted { session_id, .. })
        | InputFact::StatusDrive(StatusDrive::TurnEnded { session_id, .. })
        | InputFact::StatusDrive(StatusDrive::DistillCompleted { session_id, .. })
        | InputFact::StatusDrive(StatusDrive::ChannelsChanged { session_id, .. })
        | InputFact::StatusDrive(StatusDrive::Tick { session_id, .. })
        | InputFact::StatusDrive(StatusDrive::SessionEnded { session_id, .. }) => Some(session_id),
        _ => None,
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
    outbox: &Mutex<crate::reconcile::OutboxReconciler>,
    result: &TransactionResult<StatusCommand>,
    _preview: &TransactionResult<StatusCommand>,
) -> Vec<String> {
    let mut ids = Vec::new();
    for effect in effects {
        let status = match effect {
            StatusEffect::Publish { status, .. } | StatusEffect::Expire { status } => status,
        };
        let source_ref = format!(
            "status/{}#tx:{}",
            status.session_id.as_str(),
            result.transaction_id.get()
        );
        if let Some(id) = enqueue_status(
            provider,
            keys,
            store,
            outbox,
            status,
            source_ref,
            now_secs(),
        )
        .await
        {
            ids.push(id);
        }
    }
    ids
}

fn preview_matches(
    preview: Option<&TransactionResult<StatusCommand>>,
    committed: &TransactionResult<StatusCommand>,
) -> bool {
    let Some(preview) = preview else {
        return false;
    };
    preview.revision == committed.revision
        && crate::reconcile::preview::command_plans_match(
            preview.resource_plan.commands(),
            committed.resource_plan.commands(),
        )
}

/// Encode + sign the status and park the signed JSON on the `outbox`, returning
/// the signed event id. The drainer publishes it; the engine never talks to the
/// relay for status.
async fn enqueue_status(
    provider: &Nip29Provider,
    keys: &Keys,
    store: &Mutex<Store>,
    outbox: &Mutex<crate::reconcile::OutboxReconciler>,
    status: Status,
    source_ref: String,
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
    let event_id = signed.id.to_hex();
    let event_hash = crate::instrument::window_hash(&json);
    match store.lock() {
        Ok(g) => {
            let local_id = match g.enqueue_outbox(&json, now) {
                Ok(id) => id,
                Err(e) => {
                    tracing::error!(error = %e, "enqueue_status: enqueue_outbox failed — status not published this cycle");
                    return None;
                }
            };
            drop(g);
            if let Err(e) = crate::outbox_seam::drive(
                outbox,
                store,
                "enqueue",
                InputFact::OutboxEnqueueApplied {
                    local_id,
                    event_id: event_id.clone(),
                    event_hash,
                    source_surface: "status".into(),
                    source_ref,
                    at: now,
                },
            ) {
                tracing::error!(error = %e, "enqueue_status: outbox graph drive failed — status not published this cycle");
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
    Some(event_id)
}
