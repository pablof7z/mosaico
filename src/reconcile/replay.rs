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
    })
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum ReplaySurface {
    Status,
    Subscriptions,
    HookContext,
}

fn script_surface(script: &DataTransactionScript<InputFact>) -> Result<ReplaySurface> {
    let mut surface = None;
    for step in script.steps() {
        for operation in step.operations() {
            let next = match operation {
                InputFact::StatusDrive(_) => ReplaySurface::Status,
                InputFact::SubscriptionSync { .. } => ReplaySurface::Subscriptions,
                InputFact::HookContextRender(_) => ReplaySurface::HookContext,
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
}
