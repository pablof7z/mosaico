//! `probe acid`: verify a live `why` cause with preview counterfactuals.
use super::artifact;
use super::{required_str, DaemonState};
use crate::reconcile::journal::{InputFact, StatusDrive};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::sync::Arc;

mod status;

pub(super) fn acid_value(state: &Arc<DaemonState>, params: &Value) -> Result<Value> {
    let handle = required_str(params, "handle")?;
    let fact = artifact::fact_param(params, "fact")?.context("probe acid: requires `fact`")?;
    let causes = why_causes(state, handle)?;
    let cause = params
        .get("cause")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| causes.first().cloned())
        .context("probe acid: live why returned no input causes")?;

    let original = artifact::preview_artifact(state, &fact)?;
    let removed_fact = remove_cause(state, fact.clone(), &cause)?;
    let removed = artifact::preview_artifact(state, &removed_fact)?;
    let unrelated_fact = mutate_unrelated(fact)?;
    let unrelated = artifact::preview_artifact(state, &unrelated_fact)?;

    let removal_diff = artifact::field_diff(&original.value, &removed.value);
    let unrelated_diff = artifact::field_diff(&original.value, &unrelated.value);
    let necessary = original.hash != removed.hash;
    let unrelated_stable = original.hash == unrelated.hash;

    Ok(json!({
        "verb": "acid",
        "handle": handle,
        "surface": original.surface,
        "cause": cause,
        "input_causes": causes,
        "necessary": necessary,
        "unrelated_stable": unrelated_stable,
        "ok": necessary && unrelated_stable,
        "original_hash": original.hash,
        "removed_hash": removed.hash,
        "unrelated_hash": unrelated.hash,
        "removal_diff": removal_diff,
        "unrelated_diff": unrelated_diff,
    }))
}

fn why_causes(state: &Arc<DaemonState>, handle: &str) -> Result<Vec<String>> {
    if let Some(session) = strip_handle_id(handle, &["status:", "status/"]) {
        let r = state.status.lock().expect("status mutex poisoned");
        return Ok(r
            .explain_status(session)
            .map(|why| why.input_causes)
            .unwrap_or_default());
    }
    if let Some(channel) = handle.strip_prefix("sub:") {
        let r = state.subs.lock().expect("subs mutex poisoned");
        return Ok(r.explain_channel(channel).input_causes);
    }
    if handle.starts_with("sub/") {
        let r = state.subs.lock().expect("subs mutex poisoned");
        return Ok(r
            .explain_resource_path(handle)
            .map(|why| why.input_causes)
            .unwrap_or_default());
    }
    if let Some(session) = strip_handle_id(
        handle,
        &["turn:", "turn/", "turn_lifecycle:", "turn_lifecycle/"],
    ) {
        let r = state
            .turn_lifecycle
            .lock()
            .expect("turn lifecycle mutex poisoned");
        return Ok(r
            .explain_turn(session)
            .map(|why| why.input_causes)
            .unwrap_or_default());
    }
    if let Some(session) = strip_handle_id(handle, &["cursor:", "cursor/", "cur:", "cur/"]) {
        let r = state.cursor.lock().expect("cursor mutex poisoned");
        return Ok(r
            .explain_cursor(session)
            .map(|why| why.input_causes)
            .unwrap_or_default());
    }
    if let Some(raw) = strip_handle_id(handle, &["outbox:", "outbox/"]) {
        return super::outbox_acid::causes(state, raw);
    }
    Err(anyhow::anyhow!(
        "probe acid: handle must be `status:<session>`, `sub:<channel>`, `turn:<session>`, `cursor:<session>`, `outbox:<local_id>`, or the matching visible resource path"
    ))
}

fn strip_handle_id<'a>(handle: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes.iter().find_map(|prefix| {
        handle.strip_prefix(prefix).and_then(|rest| {
            let id = rest.split('/').next().unwrap_or(rest);
            (!id.is_empty()).then_some(id)
        })
    })
}

fn remove_cause(state: &Arc<DaemonState>, fact: InputFact, cause: &str) -> Result<InputFact> {
    match fact {
        InputFact::StatusDrive(StatusDrive::DistillCompleted {
            pubkey,
            mut title,
            mut activity,
            window_hash,
            mut at,
        }) => {
            let current = state
                .status
                .lock()
                .expect("status mutex poisoned")
                .state_rows()
                .into_iter()
                .find(|row| row.session == pubkey)
                .with_context(|| format!("probe acid: no status row for `{pubkey}`"))?;
            if cause.ends_with("/activity") {
                activity = current.activity;
            } else if cause.ends_with("/title") {
                title = current.title;
            } else if cause.ends_with("/arm") {
                at = current_status_arm_at(state, &pubkey)?;
            } else {
                anyhow::bail!("probe acid: unsupported status cause `{cause}`");
            }
            Ok(InputFact::StatusDrive(StatusDrive::DistillCompleted {
                pubkey,
                title,
                activity,
                window_hash,
                at,
            }))
        }
        InputFact::StatusDrive(StatusDrive::Tick {
            pubkey,
            automatic_delivery,
            ..
        }) => status::remove_tick_arm(state, pubkey, automatic_delivery, cause),
        InputFact::SubscriptionSync { mut snapshot, at } => {
            if let Some(session) = subscription_session_cause(cause) {
                snapshot.sessions.remove(&session);
            } else if cause == "subscriptions/daemon/channels" {
                snapshot.daemon_channels.clear();
            } else if cause == "subscriptions/daemon/addressed_pubkeys" {
                snapshot.addressed_pubkeys.clear();
            } else if cause == "subscriptions/daemon/archived_channels" {
                snapshot.archived_channels.clear();
            } else {
                anyhow::bail!("probe acid: unsupported subscription cause `{cause}`");
            }
            Ok(InputFact::SubscriptionSync { snapshot, at })
        }
        InputFact::TurnStarted { pubkey, at } => {
            if !cause.ends_with("/turn_started") {
                anyhow::bail!("probe acid: unsupported turn cause `{cause}`");
            }
            let current = current_turn_row(state, &pubkey)?;
            Ok(InputFact::TurnStarted {
                pubkey,
                at: current.turn_started_at.min(at),
            })
        }
        InputFact::TurnEnded { pubkey, .. } => {
            if !cause.ends_with("/turn_ended") {
                anyhow::bail!("probe acid: unsupported turn cause `{cause}`");
            }
            Ok(InputFact::TurnEnded { pubkey, at: 0 })
        }
        InputFact::TranscriptWindowCaptured {
            pubkey,
            window_hash,
            at,
        } => {
            if !cause.ends_with("/transcript_window") {
                anyhow::bail!("probe acid: unsupported turn cause `{cause}`");
            }
            let current = current_turn_row(state, &pubkey)?;
            Ok(InputFact::TranscriptWindowCaptured {
                pubkey,
                window_hash: current.transcript_ref.unwrap_or(window_hash),
                at,
            })
        }
        InputFact::TurnCheckRequested {
            pubkey,
            mut observed_cursor,
            mut working,
            mut at,
        } => {
            if cause.ends_with("/current_cursor") || cause.ends_with("/observed_cursor") {
                observed_cursor = observed_cursor.saturating_add(1);
            } else if cause.ends_with("/working") {
                working = false;
            } else if cause.ends_with("/now") {
                at = observed_cursor;
            } else {
                anyhow::bail!("probe acid: unsupported cursor cause `{cause}`");
            }
            Ok(InputFact::TurnCheckRequested {
                pubkey,
                observed_cursor,
                working,
                at,
            })
        }
        other => match super::outbox_acid::remove_cause(other, cause)? {
            Some(fact) => Ok(fact),
            None => Err(anyhow::anyhow!(
                "probe acid: fact/cause combination is not supported"
            )),
        },
    }
}

fn current_status_arm_at(state: &Arc<DaemonState>, pubkey: &str) -> Result<u64> {
    state
        .status
        .lock()
        .expect("status mutex poisoned")
        .current_arm_at(pubkey)
        .with_context(|| format!("probe acid: no status arm for `{pubkey}`"))
}

fn mutate_unrelated(fact: InputFact) -> Result<InputFact> {
    match fact {
        InputFact::StatusDrive(StatusDrive::DistillCompleted {
            pubkey,
            title,
            activity,
            window_hash,
            at,
        }) => Ok(InputFact::StatusDrive(StatusDrive::DistillCompleted {
            pubkey,
            title,
            activity,
            window_hash: Some(format!(
                "{}:acid-unrelated",
                window_hash.unwrap_or_else(|| "sha256".into())
            )),
            at,
        })),
        InputFact::SubscriptionSync { snapshot, at } => Ok(InputFact::SubscriptionSync {
            snapshot,
            at: at.saturating_add(999_999),
        }),
        InputFact::TranscriptWindowCaptured {
            pubkey,
            window_hash,
            at,
        } => Ok(InputFact::TranscriptWindowCaptured {
            pubkey,
            window_hash,
            at: at.saturating_add(999_999),
        }),
        InputFact::TurnStarted { pubkey, at } => Ok(InputFact::TurnStarted { pubkey, at }),
        InputFact::TurnEnded { pubkey, at } => Ok(InputFact::TurnEnded { pubkey, at }),
        InputFact::TurnCheckRequested {
            pubkey,
            observed_cursor,
            working,
            at,
        } => Ok(InputFact::TurnCheckRequested {
            pubkey,
            observed_cursor,
            working,
            at,
        }),
        other => super::outbox_acid::mutate_unrelated(other)
            .ok_or_else(|| anyhow::anyhow!("probe acid: no unrelated mutation for this fact")),
    }
}

fn current_turn_row(
    state: &Arc<DaemonState>,
    pubkey: &str,
) -> Result<crate::reconcile::turn_lifecycle::TurnStateRow> {
    state
        .turn_lifecycle
        .lock()
        .expect("turn lifecycle mutex poisoned")
        .state_rows()
        .into_iter()
        .find(|row| row.session == pubkey)
        .with_context(|| format!("probe acid: no turn_lifecycle row for `{pubkey}`"))
}

fn subscription_session_cause(cause: &str) -> Option<String> {
    let rest = cause.strip_prefix("subscriptions/session/")?;
    let (session, field) = rest.split_once('/')?;
    (field == "channels").then(|| session.to_string())
}

#[cfg(test)]
mod tests;
