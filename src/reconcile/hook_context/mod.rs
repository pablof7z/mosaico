//! Stateful presentation cache for hook/fabric-context snapshots.
//!
//! View construction is pure. This small cache only suppresses unchanged
//! snapshots and remembers whether the session has already been told about its
//! output mode.

mod receipt;

pub use receipt::{FrameKind, HookContextReceipt, Shape};

use crate::fabric_context::{assemble::assemble_view, render_view_text, FabricView, ViewInputs};

pub struct HookContextOutcome {
    pub text: Option<String>,
    pub receipt: HookContextReceipt,
    pub transaction_id: i64,
    pub revision: i64,
}

#[derive(Default)]
pub struct HookContextState {
    last_inputs: Option<ViewInputs>,
    last_view: Option<FabricView>,
    last_cursor: Option<i64>,
    last_now: Option<i64>,
    last_headless_mode: Option<bool>,
    revision: i64,
}

impl HookContextState {
    pub(crate) fn render_context(
        &mut self,
        pubkey: &str,
        kind: &str,
        cursor: i64,
        now: i64,
        inputs: ViewInputs,
    ) -> HookContextOutcome {
        let force = inputs.force();
        let view =
            assemble_view(&inputs, cursor.max(0) as u64, now.max(0) as u64).for_agent_context();
        let changed = self.last_view.as_ref() != Some(&view);
        let first = self.last_view.is_none();
        let frame = if first {
            FrameKind::Baseline
        } else if changed {
            FrameKind::Delta
        } else {
            FrameKind::Unchanged
        };
        let text = (force || changed)
            .then(|| (force || !view.is_empty()).then(|| render_view_text(&view)))
            .flatten();
        let input_causes = if changed {
            self.changed_inputs(cursor, now, &inputs)
        } else {
            Default::default()
        };

        self.last_inputs = Some(inputs);
        self.last_view = Some(view);
        self.last_cursor = Some(cursor);
        self.last_now = Some(now);
        self.revision = self.revision.saturating_add(1);

        HookContextOutcome {
            receipt: HookContextReceipt::new(
                kind,
                pubkey,
                cursor,
                now,
                frame,
                text.as_deref(),
                input_causes,
            ),
            text,
            transaction_id: self.revision,
            revision: self.revision,
        }
    }

    pub(crate) fn record_headless_mode(&mut self, headless: bool, announce_initial: bool) -> bool {
        let prior = self.last_headless_mode.replace(headless);
        match prior {
            Some(last) => last != headless,
            None => announce_initial,
        }
    }

    fn changed_inputs(&self, cursor: i64, now: i64, inputs: &ViewInputs) -> Vec<String> {
        let mut causes = Vec::new();
        if self.last_cursor != Some(cursor) {
            causes.push("cursor".to_string());
        }
        if self.last_now != Some(now) {
            causes.push("now".to_string());
        }
        let Some(previous) = &self.last_inputs else {
            causes.extend(
                [
                    "channel-meta",
                    "members",
                    "presence",
                    "messages",
                    "reactions",
                ]
                .into_iter()
                .map(str::to_string),
            );
            return causes;
        };
        if previous.meta != inputs.meta {
            causes.push("channel-meta".to_string());
        }
        if previous.members != inputs.members {
            causes.push("members".to_string());
        }
        if previous.presence != inputs.presence {
            causes.push("presence".to_string());
        }
        if previous.messages != inputs.messages {
            causes.push("messages".to_string());
        }
        if previous.reactions != inputs.reactions {
            causes.push("reactions".to_string());
        }
        causes
    }
}
