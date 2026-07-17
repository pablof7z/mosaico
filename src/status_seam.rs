//! Host seam for status policy effects.

use std::sync::Mutex;

use nostr_sdk::prelude::Keys;

use crate::domain::{DomainEvent, Status};
use crate::fabric::provider::Nip29Provider;
use crate::reconcile::{StatusEffect, StatusOutcome, StatusReconciler};
use crate::state::Store;

pub(crate) struct DriveMeta<'a> {
    pub trigger: &'a str,
}

pub(crate) async fn drive(
    status: &Mutex<StatusReconciler>,
    provider: &Nip29Provider,
    keys: &Keys,
    store: &Mutex<Store>,
    meta: DriveMeta<'_>,
    f: impl FnOnce(&mut StatusReconciler) -> StatusOutcome,
) {
    let outcome = {
        let mut policy = status.lock().expect("status policy poisoned");
        f(&mut policy)
    };
    if outcome.effects.is_empty() {
        return;
    }
    let event_ids = apply_status_effects(&outcome, provider, keys, meta.trigger).await;
    record_status_receipt(store, &outcome, &event_ids);
}

async fn apply_status_effects(
    outcome: &StatusOutcome,
    provider: &Nip29Provider,
    keys: &Keys,
    trigger: &str,
) -> Vec<String> {
    let mut ids = Vec::new();
    for effect in &outcome.effects {
        let status = match effect {
            StatusEffect::Publish { status, .. } | StatusEffect::Expire { status } => status,
        };
        let source_ref = format!(
            "status/{}#rev:{}:{trigger}",
            status.agent.pubkey, outcome.revision
        );
        if let Some(id) = enqueue_status(provider, keys, status.clone(), source_ref).await {
            ids.push(id);
        }
    }
    ids
}

fn record_status_receipt(store: &Mutex<Store>, outcome: &StatusOutcome, event_ids: &[String]) {
    let Some(artifact_ref) = event_ids.first().cloned() else {
        return;
    };
    let effects = outcome
        .effects
        .iter()
        .map(|effect| match effect {
            StatusEffect::Publish { reason, .. } => reason.as_str(),
            StatusEffect::Expire { .. } => "expire",
        })
        .collect::<Vec<_>>();
    let changed_summary = serde_json::json!({
        "pubkey": outcome.pubkey,
        "effects": effects,
    })
    .to_string();
    let row = crate::state::receipts::NewReceipt {
        surface: "status".into(),
        transaction_id: outcome.revision as i64,
        revision: outcome.revision as i64,
        changed_summary,
        commands: serde_json::to_string(&effects).unwrap_or_else(|_| "[]".into()),
        artifact_ref: Some(artifact_ref),
        created_at: crate::instrument::now_millis(),
    };
    crate::instrument::record_receipt(&store.lock().expect("store mutex poisoned"), row);
}

async fn enqueue_status(
    provider: &Nip29Provider,
    keys: &Keys,
    status: Status,
    source_ref: String,
) -> Option<String> {
    match provider.enqueue(&DomainEvent::Status(status), keys).await {
        Ok(event_id) => {
            tracing::debug!(event_id = %event_id.to_hex(), source_ref, "status accepted by NMP");
            Some(event_id.to_hex())
        }
        Err(error) => {
            tracing::error!(
                error = %format!("{error:#}"),
                source_ref,
                "status submission to NMP failed"
            );
            None
        }
    }
}
