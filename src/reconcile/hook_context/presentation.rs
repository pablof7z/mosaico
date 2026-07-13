use super::{render_view_text, FabricView, HookContextReconciler};

#[derive(Default)]
pub(super) struct RenderCache {
    pub(super) last_view: Option<FabricView>,
    pub(super) last_text: Option<String>,
    last_headless_mode: Option<bool>,
}

pub(super) fn render_text(force: bool, frame_emitted: bool, view: &FabricView) -> Option<String> {
    (force || frame_emitted)
        .then(|| (force || !view.is_empty()).then(|| render_view_text(view)))
        .flatten()
}

impl HookContextReconciler {
    /// Record the output-presentation state and report whether this hook context
    /// needs to tell the agent about it. Keeping it outside the fabric snapshot
    /// prevents unrelated chat or presence deltas from repeating the notice.
    pub(crate) fn record_headless_mode(&mut self, headless: bool) -> bool {
        let changed = self.cache.last_headless_mode != Some(headless);
        self.cache.last_headless_mode = Some(headless);
        changed
    }
}
