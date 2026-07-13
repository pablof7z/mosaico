//! Probe-facing, read-only inspection of the live hook-context graph.

use super::HookContextReconciler;

impl HookContextReconciler {
    pub fn labels(&self) -> &crate::reconcile::labels::NodeLabels {
        &self.labels
    }

    /// The live graph revision for probe/oracle reporting.
    pub fn revision(&self) -> u64 {
        self.graph.revision().get()
    }

    /// Total Trellis node count in this per-session graph.
    pub fn graph_node_count(&self) -> usize {
        self.graph.nodes().count()
    }

    /// Successful renders performed through this graph instance.
    pub fn render_count(&self) -> u64 {
        self.render_count
    }

    /// The exact text emitted by the most recent render, if it was not suppressed.
    pub fn current_text(&self) -> Option<String> {
        self.cache.last_text.clone()
    }

    /// Stable labels for the six canonical input nodes, in dependency order.
    pub fn input_labels(&self) -> Vec<String> {
        let Some(nodes) = self.nodes.as_ref() else {
            return Vec::new();
        };
        [
            nodes.cursor.id(),
            nodes.now.id(),
            nodes.meta.id(),
            nodes.members.id(),
            nodes.presence.id(),
            nodes.messages.id(),
        ]
        .into_iter()
        .filter_map(|id| self.labels.label_of(id).map(str::to_string))
        .collect()
    }

    /// Stable label for the derived fabric view node.
    pub fn view_label(&self) -> Option<String> {
        self.nodes
            .as_ref()
            .and_then(|nodes| self.labels.label_of(nodes.view.id()))
            .map(str::to_string)
    }

    /// Graph-local debug dump for `probe state hook_context <session> --dump`.
    pub fn debug_dump(&self) -> String {
        self.graph.debug_dump()
    }

    /// The derived view's latest input causes, resolved to stable labels.
    pub fn why_view_input_causes(&self) -> Vec<String> {
        self.labels.labels_for(&self.why_view_causes())
    }

    #[cfg(test)]
    pub fn set_current_text_for_test(&mut self, text: impl Into<String>) {
        self.cache.last_text = Some(text.into());
    }
}
