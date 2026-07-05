use std::fmt::Debug;

use anyhow::{Context, Result};
use trellis_testing::{DataTransactionScript, ScenarioTarget, SerializedScenario, TrellisHarness};

use super::InputFact;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayReport {
    pub surface: String,
    pub steps: usize,
    pub resource_commands: usize,
    pub output_frames: usize,
    pub trace_json: Option<String>,
}

impl ReplayReport {
    pub(crate) fn from_harness<G, C>(
        surface: &str,
        harness: &TrellisHarness<G, C>,
        export_trace: bool,
    ) -> Result<Self>
    where
        G: ScenarioTarget<C>,
        C: Clone + Debug + PartialEq,
    {
        let scenario = harness.scenario();
        let trace_json = if export_trace {
            Some(
                SerializedScenario::from_scenario(scenario)
                    .to_json()
                    .context("serializing replay trace")?,
            )
        } else {
            None
        };
        Ok(Self {
            surface: surface.to_string(),
            steps: scenario.steps().len(),
            resource_commands: scenario.resource_commands().len(),
            output_frames: scenario.output_frames().len(),
            trace_json,
        })
    }
}

pub fn replay_script_json(json: &str, export_trace: bool) -> Result<ReplayReport> {
    let script = DataTransactionScript::<InputFact>::from_json(json)
        .context("decoding replay capsule script")?;
    replay_script(&script, export_trace)
}

pub fn replay_script(
    script: &DataTransactionScript<InputFact>,
    export_trace: bool,
) -> Result<ReplayReport> {
    script_surface(script).and_then(|surface| match surface {
        ReplaySurface::Status => super::status::replay::replay_script(script, export_trace),
        ReplaySurface::Subscriptions => {
            super::subscriptions::replay::replay_script(script, export_trace)
        }
        ReplaySurface::HookContext => {
            super::hook_context::replay::replay_script(script, export_trace)
        }
        ReplaySurface::TurnLifecycle => {
            super::turn_lifecycle::replay::replay_script(script, export_trace)
        }
        ReplaySurface::Cursor => super::cursor::replay::replay_script(script, export_trace),
        ReplaySurface::Outbox => super::outbox::replay::replay_script(script, export_trace),
        ReplaySurface::SessionStart => {
            super::session_start::replay::replay_script(script, export_trace)
        }
    })
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum ReplaySurface {
    Status,
    Subscriptions,
    HookContext,
    TurnLifecycle,
    Cursor,
    Outbox,
    SessionStart,
}

fn script_surface(script: &DataTransactionScript<InputFact>) -> Result<ReplaySurface> {
    let mut surface = None;
    for step in script.steps() {
        for operation in step.operations() {
            let next = match operation {
                InputFact::StatusDrive(_) => ReplaySurface::Status,
                InputFact::SubscriptionSync { .. } => ReplaySurface::Subscriptions,
                InputFact::HookContextRender(_) => ReplaySurface::HookContext,
                InputFact::TurnStarted { .. }
                | InputFact::TurnEnded { .. }
                | InputFact::TranscriptWindowCaptured { .. } => ReplaySurface::TurnLifecycle,
                InputFact::TurnCheckRequested { .. } => ReplaySurface::Cursor,
                InputFact::OutboxEnqueueApplied { .. } | InputFact::RelayPublishAccepted { .. } => {
                    ReplaySurface::Outbox
                }
                InputFact::SessionStartRequested(_)
                | InputFact::SessionStartFailed(_)
                | InputFact::SessionStarted { .. } => ReplaySurface::SessionStart,
                other => anyhow::bail!(
                    "replay capsule operation is not a supported surface drive fact: {other:?}"
                ),
            };
            match surface {
                Some(current) if current != next => {
                    anyhow::bail!("replay capsule mixes surfaces");
                }
                Some(_) => {}
                None => surface = Some(next),
            }
        }
    }
    surface.context("replay capsule contains no operations")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_mixed_surface_scripts() {
        let mut script = DataTransactionScript::new();
        script
            .step("one")
            .operation(InputFact::SubscriptionSync {
                snapshot: Default::default(),
                at: 1,
            })
            .commit();
        script
            .step("two")
            .operation(InputFact::StatusDrive(
                crate::reconcile::StatusDrive::Tick {
                    session_id: "s1".into(),
                    at: 1,
                },
            ))
            .commit();
        let err = replay_script(&script, false).unwrap_err();
        assert!(err.to_string().contains("mixes surfaces"));
    }

    #[test]
    fn diagnosis_corpus_replay_fixtures_are_valid() {
        let leaked_close = replay_script_json(
            include_str!("../../tests/fixtures/trellis_diagnosis/leaked-close.json"),
            false,
        )
        .unwrap();
        assert_eq!(leaked_close.surface, "subscriptions");
        assert_eq!(leaked_close.steps, 2);
        assert_eq!(
            leaked_close.resource_commands, 2,
            "first owner leaving must not close a shared subscription"
        );

        let false_republish = replay_script_json(
            include_str!("../../tests/fixtures/trellis_diagnosis/false-republish.json"),
            false,
        )
        .unwrap();
        assert_eq!(false_republish.surface, "status");
        assert_eq!(false_republish.steps, 2);
        assert_eq!(
            false_republish.resource_commands, 1,
            "same-bucket unchanged tick must not republish status"
        );
    }

    #[test]
    fn turn_lifecycle_replay_accepts_canonical_turn_facts() {
        let mut script = DataTransactionScript::new();
        script
            .step("start")
            .operation(InputFact::TurnStarted {
                session_id: "s1".into(),
                at: 100,
            })
            .commit();
        script
            .step("end")
            .operation(InputFact::TurnEnded {
                session_id: "s1".into(),
                at: 130,
            })
            .commit();

        let report = replay_script(&script, false).unwrap();
        assert_eq!(report.surface, "turn_lifecycle");
        assert_eq!(report.steps, 2);
        assert_eq!(report.resource_commands, 2);
    }

    #[test]
    fn cursor_replay_accepts_canonical_turn_check_facts() {
        let mut script = DataTransactionScript::new();
        script
            .step("first")
            .operation(InputFact::TurnCheckRequested {
                session_id: "s1".into(),
                observed_cursor: 10,
                working: true,
                at: 20,
            })
            .commit();
        script
            .step("stale")
            .operation(InputFact::TurnCheckRequested {
                session_id: "s1".into(),
                observed_cursor: 10,
                working: true,
                at: 21,
            })
            .commit();

        let report = replay_script(&script, false).unwrap();
        assert_eq!(report.surface, "cursor");
        assert_eq!(report.steps, 2);
        assert_eq!(report.resource_commands, 2);
    }

    #[test]
    fn outbox_replay_accepts_enqueue_and_publish_result_facts() {
        let mut script = DataTransactionScript::new();
        script
            .step("enqueue")
            .operation(InputFact::OutboxEnqueueApplied {
                local_id: 7,
                event_id: "ev7".into(),
                event_hash: "sha256:event".into(),
                source_surface: "status".into(),
                source_ref: "status/s1#tx:1".into(),
                at: 100,
            })
            .commit();
        script
            .step("accepted")
            .operation(InputFact::RelayPublishAccepted {
                local_id: 7,
                event_id: "ev7".into(),
                accepted: true,
                error: None,
                at: 120,
            })
            .commit();

        let report = replay_script(&script, false).unwrap();
        assert_eq!(report.surface, "outbox");
        assert_eq!(report.steps, 2);
        assert_eq!(report.resource_commands, 2);
    }
}
