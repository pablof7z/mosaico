//! Host seam for the authoritative outbox graph.

use std::sync::Mutex;

use anyhow::Result;

use crate::reconcile::{InputFact, OutboxEffect, OutboxReconciler};
use crate::state::Store;

pub(crate) fn drive(
    outbox: &Mutex<OutboxReconciler>,
    store: &Mutex<Store>,
    trigger: &str,
    fact: InputFact,
) -> Result<()> {
    let start = std::time::Instant::now();
    let facts = vec![fact.clone()];
    let (preview, outcome, commit, trigger_ref) = {
        let mut r = outbox.lock().expect("outbox mutex poisoned");
        let preview = r
            .preview_fact(&fact)
            .map_err(|e| anyhow::anyhow!("outbox preview failed: {e:?}"))?
            .ok_or_else(|| anyhow::anyhow!("unsupported outbox fact"))?;
        let outcome = r
            .drive(fact)
            .map_err(|e| anyhow::anyhow!("outbox drive failed: {e:?}"))?;
        let mut commit = crate::reconcile::CommitFacts::from_result(
            r.labels(),
            &outcome.result,
            r.graph_node_count(),
        );
        commit.graph_resources = r.state_rows().len() as i64;
        let trigger_ref = outbox_trigger_ref(&outcome.result);
        (preview.result, outcome, commit, trigger_ref)
    };
    if !crate::reconcile::preview::command_plans_match(
        preview.resource_plan.commands(),
        outcome.result.resource_plan.commands(),
    ) {
        anyhow::bail!("outbox effects blocked: committed plan was not previewed first");
    }
    apply_effects(store, outcome.effects)?;
    let created_at = crate::instrument::now_millis();
    let duration_us = start.elapsed().as_micros() as i64;
    let g = store.lock().expect("store mutex poisoned");
    crate::instrument::record_commit(
        &g,
        "outbox",
        trigger,
        trigger_ref.as_deref(),
        &commit,
        duration_us,
        created_at,
    );
    crate::replay_capsules::record_many(
        &g,
        "outbox",
        trigger,
        trigger_ref.as_deref(),
        facts,
        created_at,
    );
    Ok(())
}

fn apply_effects(store: &Mutex<Store>, effects: Vec<OutboxEffect>) -> Result<()> {
    for effect in effects {
        match effect {
            OutboxEffect::None => {}
            OutboxEffect::MarkPublished { local_id } => {
                store
                    .lock()
                    .expect("store mutex poisoned")
                    .apply_outbox_projection(local_id, "published", None, false)?;
            }
            OutboxEffect::MarkFailed { local_id, error } => {
                store
                    .lock()
                    .expect("store mutex poisoned")
                    .apply_outbox_projection(local_id, "pending", Some(&error), true)?;
            }
        }
    }
    Ok(())
}

fn outbox_trigger_ref(
    result: &trellis_core::TransactionResult<crate::reconcile::outbox::OutboxCommand>,
) -> Option<String> {
    result
        .resource_plan
        .commands()
        .iter()
        .map(|c| crate::reconcile::labels::key_path(c.key()))
        .next()
}
