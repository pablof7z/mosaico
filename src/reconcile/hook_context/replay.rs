use anyhow::{Context, Result};
use trellis_core::{Graph, GraphResult, Transaction};
use trellis_testing::{DataTransactionScript, TrellisHarness};

use crate::fabric_context::{assemble::assemble_view, render_view_text, ViewInputs};
use crate::reconcile::journal::{HookContextRenderFact, InputFact};
use crate::reconcile::labels::NodeLabels;
use crate::reconcile::replay::ReplayReport;

use super::{build_nodes, Nodes};

struct ReplayState {
    nodes: Option<Nodes>,
    labels: NodeLabels,
}

impl ReplayState {
    fn new() -> Self {
        Self {
            nodes: None,
            labels: NodeLabels::new(),
        }
    }

    fn apply(&mut self, operation: &InputFact, tx: &mut Transaction<'_, ()>) -> GraphResult<()> {
        let InputFact::HookContextRender(fact) = operation else {
            return Ok(());
        };
        let inputs = decode_inputs(fact).expect("hook replay fact was prevalidated");
        let nodes = match self.nodes.take() {
            Some(nodes) => nodes,
            None => build_nodes(tx, &fact.session_id, &mut self.labels)?,
        };
        tx.set_input(nodes.cursor, fact.cursor)?;
        tx.set_input(nodes.now, fact.now)?;
        tx.set_input(nodes.meta, inputs.meta)?;
        tx.set_input(nodes.members, inputs.members)?;
        tx.set_input(nodes.presence, inputs.presence)?;
        tx.set_input(nodes.messages, inputs.messages)?;
        self.nodes = Some(nodes);
        Ok(())
    }
}

pub(crate) fn replay_script(
    script: &DataTransactionScript<InputFact>,
    export_trace: bool,
) -> Result<ReplayReport> {
    validate_script(script)?;
    let first = run(script).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let second = run(script).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    first
        .assert_replay_matches(&second)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    ReplayReport::from_harness("hook_context", &first, export_trace)
}

fn run(
    script: &DataTransactionScript<InputFact>,
) -> Result<TrellisHarness<Graph<()>, ()>, trellis_testing::ScenarioError> {
    let mut state = ReplayState::new();
    TrellisHarness::replay_data(Graph::<()>::new, script, move |operation, tx| {
        state.apply(operation, tx)
    })
}

fn validate_script(script: &DataTransactionScript<InputFact>) -> Result<()> {
    for step in script.steps() {
        for operation in step.operations() {
            if let InputFact::HookContextRender(fact) = operation {
                validate_fact(fact)
                    .with_context(|| format!("validating hook replay step `{}`", step.name()))?;
            }
        }
    }
    Ok(())
}

fn validate_fact(fact: &HookContextRenderFact) -> Result<()> {
    let inputs = decode_inputs(fact)?;
    if inputs.force() != fact.force {
        anyhow::bail!("hook replay force flag does not match ViewInputs");
    }
    let view = assemble_view(&inputs, fact.cursor.max(0) as u64, fact.now.max(0) as u64);
    let text = (fact.force || !view.is_empty()).then(|| render_view_text(&view));
    let reproduced_hash = text.as_deref().map(crate::replay_capsules::text_hash);
    if reproduced_hash != fact.emitted_text_hash {
        anyhow::bail!(
            "hook replay emitted text hash mismatch: capsule={:?} replay={:?}",
            fact.emitted_text_hash,
            reproduced_hash
        );
    }
    Ok(())
}

fn decode_inputs(fact: &HookContextRenderFact) -> Result<ViewInputs> {
    serde_json::from_value(fact.inputs_json.clone()).context("decoding ViewInputs")
}

#[cfg(test)]
mod tests {
    use crate::fabric_context::{render_fabric_context, FabricContextInput};
    use crate::reconcile::hook_context::HookContextReconciler;
    use crate::state::Store;

    use super::*;

    #[test]
    fn hook_capsule_validates_text_hash_and_replays() {
        let store = Store::open_memory().unwrap();
        let input = FabricContextInput {
            session: None,
            scope: "root",
            cursor: 0,
            now: 100,
            self_slug: "",
            self_pubkey: "",
            local_host: "host",
            forced_messages: &[],
            warnings: &[],
            force: true,
        };
        let inputs = crate::fabric_context::capture_inputs(&store, &input);
        let text = render_fabric_context(&store, input).unwrap_or_default();
        let mut r = HookContextReconciler::new();
        r.render_context("s1", "turn_start", 0, 100, inputs.clone())
            .unwrap();

        let mut script = DataTransactionScript::new();
        script
            .step("hook")
            .operation(InputFact::HookContextRender(HookContextRenderFact {
                session_id: "s1".into(),
                hook_kind: "turn_start".into(),
                cursor: 0,
                now: 100,
                force: true,
                emitted_text_hash: Some(crate::replay_capsules::text_hash(&text)),
                inputs_json: serde_json::to_value(&inputs).unwrap(),
            }))
            .commit();

        let report = replay_script(&script, true).unwrap();
        assert_eq!(report.surface, "hook_context");
        assert_eq!(report.output_frames, 1);
        assert!(report.trace_json.is_some());
    }
}
