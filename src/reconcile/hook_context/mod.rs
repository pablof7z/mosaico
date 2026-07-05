//! The hook/fabric-context snapshot as a Trellis derived node → materialized
//! output frame over DECLARED inputs.
//!
//! It replaces the hand-rolled hook audit with one derived [`FabricView`] over
//! explicit store inputs plus cursor/now, so the injected bytes and receipt share
//! the same dependency trace.

mod receipt;
#[cfg(test)]
mod tests;

pub use receipt::{FrameKind, HookContextReceipt, Shape};

use trellis_core::{
    AuditExplanationLevel, DependencyList, DerivedNode, Graph, GraphResult, InputNode, NodeId,
    OutputKey, TransactionOptions,
};

use crate::fabric_context::{
    assemble::assemble_view, render_view_text, FabricView, MembersInput, MessagesInput, MetaInput,
    PresenceInput, ViewInputs,
};
use crate::reconcile::labels::{CommitFacts, NodeLabels};

/// One render's product: the byte-exact snapshot (suppressed to `None` when empty
/// and unforced) plus the graph-sourced receipt — the instrumentation seam a
/// later `explain` CLI persists and replays.
pub struct HookContextOutcome {
    pub text: Option<String>,
    pub receipt: HookContextReceipt,
    pub transaction_id: i64,
    pub revision: i64,
    pub commit: CommitFacts,
}

/// Per-graph handles for the reusable snapshot node-set.
struct Nodes {
    cursor: InputNode<i64>,
    now: InputNode<i64>,
    meta: InputNode<MetaInput>,
    members: InputNode<MembersInput>,
    presence: InputNode<PresenceInput>,
    messages: InputNode<MessagesInput>,
    view: DerivedNode<FabricView>,
    output: OutputKey,
}

/// Reconciler that derives the fabric snapshot as a materialized output frame.
pub struct HookContextReconciler {
    graph: Graph<()>,
    nodes: Option<Nodes>,
    /// Last derived view, so an unchanged commit (no frame) still yields bytes.
    last_view: Option<FabricView>,
    labels: NodeLabels,
}

impl Default for HookContextReconciler {
    fn default() -> Self {
        Self::new()
    }
}

impl HookContextReconciler {
    /// Build an empty reconciler; the node-set is created on first render.
    pub fn new() -> Self {
        Self {
            graph: Graph::<()>::new(),
            nodes: None,
            last_view: None,
            labels: NodeLabels::new(),
        }
    }

    pub fn labels(&self) -> &NodeLabels {
        &self.labels
    }

    /// Render the snapshot for a session over the canonical inputs plus the
    /// explicit `cursor`/`now`. Returns the byte-exact text and the graph-sourced
    /// receipt. This is the single authority that produces AND explains the
    /// injected snapshot, so the two cannot drift.
    pub(crate) fn render_context(
        &mut self,
        session_id: &str,
        kind: &str,
        cursor: i64,
        now: i64,
        inputs: ViewInputs,
    ) -> GraphResult<HookContextOutcome> {
        let force = inputs.force();
        let mut tx = self.graph.begin_transaction_with_options(opts())?;
        // Create the node-set in the SAME transaction as the first render so its
        // real inputs land in one commit → a single Baseline frame (rather than an
        // empty baseline during setup followed by a Delta).
        let nodes = match self.nodes.take() {
            Some(nodes) => nodes,
            None => build_nodes(&mut tx, session_id, &mut self.labels)?,
        };
        tx.set_input(nodes.cursor, cursor)?;
        tx.set_input(nodes.now, now)?;
        tx.set_input(nodes.meta, inputs.meta)?;
        tx.set_input(nodes.members, inputs.members)?;
        tx.set_input(nodes.presence, inputs.presence)?;
        tx.set_input(nodes.messages, inputs.messages)?;
        let result = tx.commit()?;
        drop(tx);
        let mut commit =
            CommitFacts::from_result(&self.labels, &result, self.graph.nodes().count());
        self.nodes = Some(nodes);
        let nodes = self.nodes.as_ref().expect("nodes present");
        commit.graph_resources = 1;

        let output_key = nodes.output;
        let transaction_id = result.transaction_id.get() as i64;
        let revision = result.revision.get() as i64;
        // The frame carries the derived view; an unchanged commit emits none, so
        // fall back to the cached last view (identical by construction).
        let frame = result
            .output_frames
            .iter()
            .find(|f| f.output_key == output_key);
        if let Some(view) = frame.and_then(|f| f.kind.payload::<FabricView>()) {
            self.last_view = Some(view.clone());
        }
        let view = self
            .last_view
            .clone()
            .expect("a view was materialized at least once");

        let text = (force || !view.is_empty()).then(|| render_view_text(&view));
        // Attribute from THIS transaction's frame only: `why_output_frame` retains
        // the previous explanation across an unchanged commit, so gate on whether a
        // frame was actually emitted now.
        let frame_kind = FrameKind::from_output_kind(frame.map(|f| &f.kind));
        let input_causes = if frame.is_some() {
            self.input_cause_labels(output_key)
        } else {
            Vec::new()
        };
        let receipt = HookContextReceipt::new(
            kind,
            session_id,
            cursor,
            now,
            frame_kind,
            text.as_deref(),
            input_causes,
        );
        Ok(HookContextOutcome {
            text,
            receipt,
            transaction_id,
            revision,
            commit,
        })
    }

    /// Map the frame's canonical input causes to stable labels for the receipt.
    fn input_cause_labels(&self, output_key: OutputKey) -> Vec<String> {
        let Some(expl) = self.graph.why_output_frame(output_key) else {
            return Vec::new();
        };
        expl.input_causes
            .iter()
            .filter_map(|id| self.label_for(*id))
            .map(str::to_string)
            .collect()
    }

    /// Stable label for a canonical input node id (receipt/attribution).
    fn label_for(&self, id: NodeId) -> Option<&'static str> {
        let n = self.nodes.as_ref()?;
        Some(match id {
            _ if id == n.cursor.id() => "cursor",
            _ if id == n.now.id() => "now",
            _ if id == n.meta.id() => "channel-meta",
            _ if id == n.members.id() => "members",
            _ if id == n.presence.id() => "presence",
            _ if id == n.messages.id() => "messages",
            _ => return None,
        })
    }

    /// The presence input node id — instrumentation asserts a "why is @X working"
    /// snapshot change is attributed to it.
    pub fn presence_input(&self) -> Option<NodeId> {
        self.nodes.as_ref().map(|n| n.presence.id())
    }

    /// The cursor input node id — instrumentation asserts the shape decision is
    /// attributed to it.
    pub fn cursor_input(&self) -> Option<NodeId> {
        self.nodes.as_ref().map(|n| n.cursor.id())
    }

    /// The derived view node's input causes — the "why did the snapshot change"
    /// trace, sourced from the render itself.
    pub fn why_view_causes(&self) -> Vec<NodeId> {
        self.nodes
            .as_ref()
            .and_then(|n| self.graph.why_changed(n.view))
            .map(|e| e.input_causes.clone())
            .unwrap_or_default()
    }

    /// The full-recompute oracle: incremental state must equal a rebuild.
    pub fn assert_oracle(&self) -> GraphResult<()> {
        self.graph.assert_incremental_equals_full()?;
        Ok(())
    }
}

/// Stage the reusable node-set (scope, six inputs, derived view, output) inside a
/// caller-owned transaction — the first render commits it together with real
/// inputs so one Baseline frame carries the actual snapshot.
fn build_nodes(
    tx: &mut trellis_core::Transaction<'_, ()>,
    session_id: &str,
    labels: &mut NodeLabels,
) -> GraphResult<Nodes> {
    let scope = tx.create_scope("hook-context")?;

    let cursor = tx.input::<i64>("cursor")?;
    labels.record(cursor.id(), format!("hook/{session_id}/cursor"));
    tx.set_input(cursor, 0)?;
    let now = tx.input::<i64>("now")?;
    labels.record(now.id(), format!("hook/{session_id}/now"));
    tx.set_input(now, 0)?;
    let meta = tx.input::<MetaInput>("channel-meta")?;
    labels.record(meta.id(), format!("hook/{session_id}/channel-meta"));
    tx.set_input(meta, MetaInput::default())?;
    let members = tx.input::<MembersInput>("members")?;
    labels.record(members.id(), format!("hook/{session_id}/members"));
    tx.set_input(members, MembersInput::default())?;
    let presence = tx.input::<PresenceInput>("presence")?;
    labels.record(presence.id(), format!("hook/{session_id}/presence"));
    tx.set_input(presence, PresenceInput::default())?;
    let messages = tx.input::<MessagesInput>("messages")?;
    labels.record(messages.id(), format!("hook/{session_id}/messages"));
    tx.set_input(messages, MessagesInput::default())?;

    let view = tx.derived(
        "fabric-view",
        DependencyList::new([
            cursor.id(),
            now.id(),
            meta.id(),
            members.id(),
            presence.id(),
            messages.id(),
        ])?,
        move |ctx| {
            let inputs = ViewInputs::from_parts(
                ctx.input(meta)?.clone(),
                ctx.input(members)?.clone(),
                ctx.input(presence)?.clone(),
                ctx.input(messages)?.clone(),
            );
            Ok(assemble_view(
                &inputs,
                (*ctx.input(cursor)?).max(0) as u64,
                (*ctx.input(now)?).max(0) as u64,
            ))
        },
    )?;
    labels.record(view.id(), format!("hook/{session_id}/view"));

    let output = tx.materialized_output(
        "hook-context-snapshot",
        scope,
        DependencyList::new([view.id()])?,
        move |ctx| Ok(ctx.derived(view)?.clone()),
    )?;

    Ok(Nodes {
        cursor,
        now,
        meta,
        members,
        presence,
        messages,
        view,
        output: output.key(),
    })
}

/// Transaction options with dependency-path audit so a frame can be attributed
/// to the exact canonical input (e.g. `presence`, `cursor`) that produced it.
fn opts() -> TransactionOptions {
    TransactionOptions::default().with_audit_explanations(AuditExplanationLevel::DependencyPaths)
}
