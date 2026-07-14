//! `probe diff`: counterfactual artifact comparison for preview/replay plans.

use super::artifact;
use super::{required_str, DaemonState};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::sync::Arc;
use trellis_testing::DataTransactionScript;

pub(super) fn diff_value(state: &Arc<DaemonState>, params: &Value) -> Result<Value> {
    if params
        .get("capsule")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .is_some()
    {
        diff_capsule(state, params)
    } else {
        diff_live_preview(state, params)
    }
}

fn diff_live_preview(state: &Arc<DaemonState>, params: &Value) -> Result<Value> {
    let fact = artifact::fact_param(params, "fact")?
        .context("probe diff: live preview requires `fact`")?;
    let after = artifact::preview_artifact(state, &fact)?;
    let before = artifact::empty_artifact(after.surface);
    Ok(diff_response("live-preview", None, before, after))
}

fn diff_capsule(state: &Arc<DaemonState>, params: &Value) -> Result<Value> {
    let capsule = required_str(params, "capsule")?;
    let id: i64 = capsule
        .parse()
        .with_context(|| format!("probe diff: capsule id must be an integer, got `{capsule}`"))?;
    let mutation = artifact::fact_param(params, "mutate_fact")?
        .or(artifact::fact_param(params, "fact")?)
        .context("probe diff: capsule mode requires `fact` or `mutate_fact`")?;
    let row = state
        .with_store(|s| s.get_replay_capsule(id))?
        .with_context(|| format!("probe diff: capsule {id} not found"))?;
    let script = DataTransactionScript::from_json(&row.script_json)
        .context("probe diff: decoding capsule script")?;
    let mutated = artifact::replace_last_operation(&script, mutation)?;
    let before = artifact::replay_artifact(&script)?;
    let after = artifact::replay_artifact(&mutated)?;
    Ok(diff_response("capsule-replay", Some(id), before, after))
}

fn diff_response(
    mode: &'static str,
    capsule: Option<i64>,
    before: artifact::Artifact,
    after: artifact::Artifact,
) -> Value {
    let fields = artifact::field_diff(&before.value, &after.value);
    json!({
        "verb": "diff",
        "mode": mode,
        "capsule": capsule,
        "surface": after.surface,
        "artifact_changed": before.hash != after.hash,
        "before_hash": before.hash,
        "after_hash": after.hash,
        "field_diff": fields,
        "before": before.value,
        "after": after.value,
        "ok": true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconcile::{CoverageSnapshot, InputFact, StatusDrive};
    use std::collections::{BTreeMap, BTreeSet};

    #[tokio::test]
    async fn live_status_diff_reports_changed_artifact_without_mutating() {
        let state = DaemonState::new_for_test().await;
        seed_status(&state);
        let before = state.status.lock().unwrap().revision();
        let fact = InputFact::StatusDrive(StatusDrive::DistillCompleted {
            pubkey: "s1".into(),
            title: "T".into(),
            activity: "reviewing".into(),
            window_hash: Some("sha256:w".into()),
            at: 130,
        });
        let v = diff_value(
            &state,
            &json!({ "verb": "diff", "surface": "status", "fact": fact }),
        )
        .unwrap();
        assert_eq!(v["artifact_changed"], true);
        assert!(v["field_diff"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["field"] == "commands"));
        assert_eq!(state.status.lock().unwrap().revision(), before);
    }

    #[tokio::test]
    async fn live_subscription_diff_reports_changed_artifact() {
        let state = DaemonState::new_for_test().await;
        let mut sessions = BTreeMap::new();
        sessions.insert("s1".to_string(), BTreeSet::from(["room".to_string()]));
        let fact = InputFact::SubscriptionSync {
            snapshot: CoverageSnapshot {
                daemon_channels: BTreeSet::new(),
                addressed_pubkeys: BTreeSet::new(),
                archived_channels: BTreeSet::new(),
                sessions,
            },
            at: 100,
        };
        let v = diff_value(
            &state,
            &json!({ "verb": "diff", "surface": "subscriptions", "fact": fact }),
        )
        .unwrap();
        assert_eq!(v["surface"], "subscriptions");
        assert_eq!(v["artifact_changed"], true);
    }

    #[tokio::test]
    async fn capsule_diff_replays_original_and_mutated_script() {
        let state = DaemonState::new_for_test().await;
        let mut script = DataTransactionScript::new();
        script
            .step("start")
            .operation(InputFact::StatusDrive(StatusDrive::SessionStarted(
                crate::reconcile::StatusSessionStartedArgs {
                    pubkey: "s1".into(),
                    host: "host".into(),
                    slug: "agent".into(),
                    rel_cwd: ".".into(),
                    channels: BTreeSet::from(["room".to_string()]),
                    working: true,
                    title: "T".into(),
                    activity: "reading".into(),
                    dispatch_event: None,
                    at: 100,
                },
            )))
            .commit();
        script
            .step("distill")
            .operation(InputFact::StatusDrive(StatusDrive::DistillCompleted {
                pubkey: "s1".into(),
                title: "T".into(),
                activity: "reviewing".into(),
                window_hash: Some("sha256:a".into()),
                at: 130,
            }))
            .commit();
        let id = state
            .with_store(|s| {
                s.record_replay_capsule(&crate::state::trellis_replay_capsules::NewReplayCapsule {
                    surface: "status".into(),
                    trigger_kind: "distill".into(),
                    trigger_ref: "s1".into(),
                    script_json: script.to_json().unwrap(),
                    format_version: 1,
                    created_at: 130,
                })
            })
            .unwrap();
        let mutation = InputFact::StatusDrive(StatusDrive::Tick {
            pubkey: "s1".into(),
            at: 100,
        });

        let v = diff_value(
            &state,
            &json!({ "verb": "diff", "capsule": id.to_string(), "fact": mutation }),
        )
        .unwrap();
        assert_eq!(v["mode"], "capsule-replay");
        assert_eq!(v["artifact_changed"], true);
    }

    fn seed_status(state: &Arc<DaemonState>) {
        let mut r = state.status.lock().unwrap();
        r.on_session_started(
            "s1",
            "host",
            "agent",
            ".",
            BTreeSet::from(["room".to_string()]),
            true,
            "T",
            "reading",
            100,
        )
        .unwrap();
    }
}
