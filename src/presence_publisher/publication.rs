use std::sync::{Arc, Mutex};
use std::time::Duration;

use nostr::key::Keys;

use super::{PendingPublishJobs, StatusOutcome};
use crate::domain::Status;
use crate::fabric::provider::{ConfirmedGroupScope, Nip29Provider};
use crate::reconcile::StatusEffect;
use crate::state::Store;

const PUBLISH_ATTEMPTS: usize = 3;
const RETRY_DELAY: Duration = Duration::from_millis(250);

pub(super) async fn apply_status_effects(
    outcome: &StatusOutcome,
    provider: &Nip29Provider,
    keys: &Keys,
    trigger: &str,
    confirmed_scope: Option<&ConfirmedGroupScope>,
    pending: &Arc<Mutex<PendingPublishJobs>>,
) -> Vec<String> {
    let mut ids = Vec::new();
    for effect in &outcome.effects {
        let (status, require_member) = match effect {
            StatusEffect::Publish { status, .. } => (status, true),
            StatusEffect::Expire { status } => (status, false),
        };
        let source_ref = format!(
            "status/{}#rev:{}:{trigger}",
            status.agent.pubkey, outcome.revision
        );
        for attempt in 1..=PUBLISH_ATTEMPTS {
            match enqueue_status(provider, keys, status, require_member, confirmed_scope).await {
                Ok(event_id) => {
                    tracing::debug!(
                        event_id = %event_id,
                        source_ref,
                        "status accepted by relay"
                    );
                    ids.push(event_id);
                    break;
                }
                Err(error) => {
                    let superseded = pending
                        .lock()
                        .expect("presence publish queue poisoned")
                        .supersedes(&status.agent.pubkey, outcome.revision);
                    if superseded || attempt == PUBLISH_ATTEMPTS {
                        tracing::warn!(
                            error = %format!("{error:#}"),
                            source_ref,
                            attempt,
                            superseded,
                            "status publication stopped"
                        );
                        break;
                    }
                    tokio::time::sleep(RETRY_DELAY).await;
                }
            }
        }
    }
    ids
}

async fn enqueue_status(
    provider: &Nip29Provider,
    keys: &Keys,
    status: &Status,
    require_member: bool,
    confirmed_scope: Option<&ConfirmedGroupScope>,
) -> anyhow::Result<String> {
    let event_id = match confirmed_scope {
        Some(scope) => {
            provider
                .enqueue_status_after_confirmed_scope(status, keys, scope, require_member)
                .await?
        }
        None => {
            provider
                .enqueue_status(status, keys, require_member, None)
                .await?
        }
    };
    Ok(event_id.to_hex())
}

pub(super) fn record_status_receipt(
    store: &Mutex<Store>,
    outcome: &StatusOutcome,
    event_ids: &[String],
) {
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
