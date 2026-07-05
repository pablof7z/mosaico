use super::DaemonState;
use crate::reconcile::journal::InputFact;
use anyhow::Result;
use std::sync::Arc;

pub(super) fn causes(state: &Arc<DaemonState>, raw: &str) -> Result<Vec<String>> {
    let local_id = raw
        .parse::<i64>()
        .map_err(|_| anyhow::anyhow!("probe acid: invalid outbox local id `{raw}`"))?;
    let r = state.outbox.lock().expect("outbox mutex poisoned");
    Ok(r.explain_outbox(local_id)
        .map(|why| why.input_causes)
        .unwrap_or_default())
}

pub(super) fn remove_cause(fact: InputFact, cause: &str) -> Result<Option<InputFact>> {
    match fact {
        InputFact::OutboxEnqueueApplied {
            local_id,
            mut event_id,
            mut event_hash,
            source_surface,
            mut source_ref,
            at,
        } => {
            if cause.ends_with("/event_id") {
                event_id.push_str(":acid");
            } else if cause.ends_with("/event_hash") {
                event_hash.push_str(":acid");
            } else if cause.ends_with("/source_ref") {
                source_ref.push_str(":acid");
            } else {
                anyhow::bail!("probe acid: unsupported outbox enqueue cause `{cause}`");
            }
            Ok(Some(InputFact::OutboxEnqueueApplied {
                local_id,
                event_id,
                event_hash,
                source_surface,
                source_ref,
                at,
            }))
        }
        InputFact::RelayPublishAccepted {
            local_id,
            event_id,
            mut accepted,
            mut error,
            at,
        } => {
            if cause.ends_with("/result") {
                accepted = !accepted;
                if !accepted && error.is_none() {
                    error = Some("acid counterfactual".into());
                }
            } else if cause.ends_with("/error") {
                error = Some("acid counterfactual".into());
            } else if cause.ends_with("/event_id") {
                return Ok(Some(InputFact::RelayPublishAccepted {
                    local_id,
                    event_id: format!("{event_id}:acid"),
                    accepted,
                    error,
                    at,
                }));
            } else {
                anyhow::bail!("probe acid: unsupported outbox result cause `{cause}`");
            }
            Ok(Some(InputFact::RelayPublishAccepted {
                local_id,
                event_id,
                accepted,
                error,
                at,
            }))
        }
        _ => Ok(None),
    }
}

pub(super) fn mutate_unrelated(fact: InputFact) -> Option<InputFact> {
    match fact {
        InputFact::OutboxEnqueueApplied { .. } | InputFact::RelayPublishAccepted { .. } => {
            Some(fact)
        }
        _ => None,
    }
}
