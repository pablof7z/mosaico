//! Target-specific evidence collection for non-Trellis validation targets.
use super::report::bool_at;
use super::{
    alias, awareness, channel, commit, coverage, cursor, event, hook_context, identity, inbox,
    joined, llm, membership, message, outbox, quarantine, readiness_attempt, recipient, session,
    session_start, session_watch, status, subscription, turn, workspace, DaemonState,
};
use serde_json::Value;
use std::sync::Arc;
#[derive(Default)]
pub(super) struct TargetChecks {
    pub(super) channel_evidence: Option<Value>,
    pub(super) commit_evidence: Option<Value>,
    pub(super) coverage_evidence: Option<Value>,
    pub(super) alias_evidence: Option<Value>,
    pub(super) workspace_evidence: Option<Value>,
    pub(super) membership_evidence: Option<Value>,
    pub(super) membership_snapshot_evidence: Option<Value>,
    pub(super) awareness_evidence: Option<Value>,
    pub(super) event_evidence: Option<Value>,
    pub(super) inbox_evidence: Option<Value>,
    pub(super) joined_evidence: Option<Value>,
    pub(super) quarantine_evidence: Option<Value>,
    pub(super) message_evidence: Option<Value>,
    pub(super) recipient_evidence: Option<Value>,
    pub(super) readiness_attempt_evidence: Option<Value>,
    pub(super) identity_evidence: Option<Value>,
    pub(super) hook_context_evidence: Option<Value>,
    pub(super) llm_evidence: Option<Value>,
    pub(super) txn_evidence: Option<Value>,
    pub(super) receipt_evidence: Option<Value>,
    pub(super) subscription_evidence: Option<Value>,
    pub(super) turn_evidence: Option<Value>,
    pub(super) cursor_evidence: Option<Value>,
    pub(super) session_evidence: Option<Value>,
    pub(super) status_evidence: Option<Value>,
    pub(super) outbox_evidence: Option<Value>,
    pub(super) session_start_evidence: Option<Value>,
    pub(super) session_watch_evidence: Option<Value>,
}

impl TargetChecks {
    pub(super) fn collect(
        state: &Arc<DaemonState>,
        params: &Value,
        target: Option<&str>,
        malformed: bool,
    ) -> Self {
        let Some(target) = target.filter(|_| !malformed) else {
            return Self::default();
        };
        Self {
            channel_evidence: super::target::channel_target(target)
                .map(|id| channel::channel_evidence(state, target, id)),
            commit_evidence: commit::commit_target(target)
                .map(|id| commit::commit_evidence(state, target, id)),
            coverage_evidence: coverage::coverage_target(target)
                .map(|parsed| coverage::coverage_evidence(state, target, &parsed)),
            alias_evidence: alias::alias_target(target)
                .map(|parsed| alias::alias_evidence(state, target, &parsed)),
            workspace_evidence: workspace::workspace_target(target)
                .map(|id| workspace::workspace_evidence(state, target, id)),
            membership_evidence: membership::membership_target(target)
                .map(|parsed| membership::membership_evidence(state, target, &parsed)),
            membership_snapshot_evidence: membership::membership_snapshot_target(target)
                .map(|parsed| membership::membership_snapshot_evidence(state, target, &parsed)),
            awareness_evidence: super::target::awareness_target(target)
                .map(|id| awareness::awareness_evidence(state, params, target, id)),
            event_evidence: event::event_target(target)
                .map(|id| event::event_evidence(state, target, id)),
            inbox_evidence: inbox::inbox_target(target)
                .map(|parsed| inbox::inbox_evidence(state, target, &parsed)),
            joined_evidence: joined::joined_target(target)
                .map(|parsed| joined::joined_evidence(state, target, &parsed)),
            quarantine_evidence: quarantine::quarantine_target(target)
                .map(|id| quarantine::quarantine_evidence(state, target, id)),
            message_evidence: message::message_target(target)
                .map(|id| message::message_evidence(state, target, id)),
            recipient_evidence: recipient::recipient_target(target)
                .map(|parsed| recipient::recipient_evidence(state, target, &parsed)),
            readiness_attempt_evidence: readiness_attempt::readiness_attempt_target(target)
                .map(|id| readiness_attempt::readiness_attempt_evidence(state, target, id)),
            identity_evidence: identity::identity_target(target)
                .map(|parsed| identity::identity_evidence(state, target, &parsed)),
            hook_context_evidence: hook_context::hook_context_target(target)
                .map(|id| hook_context::hook_context_evidence(state, target, id)),
            llm_evidence: llm::llm_target(target).map(|id| llm::llm_evidence(state, target, id)),
            txn_evidence: super::txn::txn_target(target).map(|txn| {
                super::txn::txn_evidence(state, target, &txn.surface, txn.transaction_id, txn.at)
            }),
            receipt_evidence: super::receipt::receipt_target(target)
                .map(|id| super::receipt::receipt_evidence(state, target, id)),
            subscription_evidence: subscription::subscription_target(target)
                .map(|sub| subscription::subscription_evidence(state, target, &sub)),
            turn_evidence: turn::turn_target(target)
                .map(|id| turn::turn_evidence(state, target, id)),
            cursor_evidence: cursor::cursor_target(target)
                .map(|id| cursor::cursor_evidence(state, target, id)),
            session_evidence: session::session_target(target)
                .map(|id| session::session_evidence(state, target, id)),
            status_evidence: status::status_target(target)
                .map(|id| status::status_evidence(state, target, id)),
            outbox_evidence: outbox::outbox_target(target)
                .map(|id| outbox::outbox_evidence(state, target, id)),
            session_start_evidence: session_start::session_start_target(target)
                .map(|id| session_start::session_start_evidence(state, target, id)),
            session_watch_evidence: session_watch::session_watch_target(target)
                .map(|id| session_watch::session_watch_evidence(state, target, id)),
        }
    }

    pub(super) fn supported(&self) -> bool {
        self.channel_evidence.is_some()
            || self.commit_evidence.is_some()
            || self.coverage_evidence.is_some()
            || self.alias_evidence.is_some()
            || self.workspace_evidence.is_some()
            || self.membership_evidence.is_some()
            || self.membership_snapshot_evidence.is_some()
            || self.awareness_evidence.is_some()
            || self.event_evidence.is_some()
            || self.inbox_evidence.is_some()
            || self.joined_evidence.is_some()
            || self.quarantine_evidence.is_some()
            || self.message_evidence.is_some()
            || self.recipient_evidence.is_some()
            || self.readiness_attempt_evidence.is_some()
            || self.identity_evidence.is_some()
            || self.hook_context_evidence.is_some()
            || self.llm_evidence.is_some()
            || self.txn_evidence.is_some()
            || self.receipt_evidence.is_some()
            || self.subscription_evidence.is_some()
            || self.turn_evidence.is_some()
            || self.cursor_evidence.is_some()
            || self.session_evidence.is_some()
            || self.status_evidence.is_some()
            || self.outbox_evidence.is_some()
            || self.session_start_evidence.is_some()
            || self.session_watch_evidence.is_some()
    }

    pub(super) fn surface_hint(&self) -> Option<&str> {
        if self.status_evidence.is_some() {
            return Some("status");
        }
        if self.subscription_evidence.is_some() {
            return Some("subscriptions");
        }
        if self.hook_context_evidence.is_some() {
            return Some("hook_context");
        }
        if self.turn_evidence.is_some() {
            return Some("turn_lifecycle");
        }
        if self.cursor_evidence.is_some() {
            return Some("cursor");
        }
        if self.outbox_evidence.is_some() {
            return Some("outbox");
        }
        if self.session_start_evidence.is_some() {
            return Some("session_start");
        }
        if self.session_watch_evidence.is_some() {
            return Some("session_watch");
        }
        self.commit_evidence
            .as_ref()
            .or(self.receipt_evidence.as_ref())
            .or(self.txn_evidence.as_ref())
            .and_then(|v| v.get("surface").and_then(Value::as_str))
            .or_else(|| self.event_receipt_surface())
    }
    pub(super) fn global_seams_checked(&self) -> bool {
        self.coverage_evidence
            .as_ref()
            .is_some_and(|v| v.get("kind").and_then(Value::as_str) == Some("validation_coverage"))
    }
    pub(super) fn push_checks(&self, checks: &mut Vec<Value>, limitations: &mut Vec<String>) {
        if let Some(v) = &self.channel_evidence {
            channel::push_channel_check(checks, limitations, v);
        }
        if let Some(v) = &self.commit_evidence {
            commit::push_commit_check(checks, limitations, v);
        }
        if let Some(v) = &self.coverage_evidence {
            coverage::push_coverage_check(checks, limitations, v);
        }
        if let Some(v) = &self.alias_evidence {
            alias::push_alias_check(checks, limitations, v);
        }
        if let Some(v) = &self.workspace_evidence {
            workspace::push_workspace_check(checks, limitations, v);
        }
        if let Some(v) = &self.membership_evidence {
            membership::push_membership_check(checks, limitations, v);
        }
        if let Some(v) = &self.membership_snapshot_evidence {
            membership::push_membership_snapshot_check(checks, limitations, v);
        }
        if let Some(v) = &self.awareness_evidence {
            awareness::push_awareness_check(checks, limitations, v);
        }
        if let Some(v) = &self.event_evidence {
            event::push_event_check(checks, limitations, v);
        }
        if let Some(v) = &self.inbox_evidence {
            inbox::push_inbox_check(checks, limitations, v);
        }
        if let Some(v) = &self.joined_evidence {
            joined::push_joined_check(checks, limitations, v);
        }
        if let Some(v) = &self.quarantine_evidence {
            quarantine::push_quarantine_check(checks, limitations, v);
        }
        if let Some(v) = &self.message_evidence {
            message::push_message_check(checks, limitations, v);
        }
        if let Some(v) = &self.recipient_evidence {
            recipient::push_recipient_check(checks, limitations, v);
        }
        if let Some(v) = &self.readiness_attempt_evidence {
            readiness_attempt::push_readiness_attempt_check(checks, limitations, v);
        }
        if let Some(v) = &self.identity_evidence {
            identity::push_identity_check(checks, limitations, v);
        }
        if let Some(v) = &self.hook_context_evidence {
            hook_context::push_hook_context_check(checks, limitations, v);
        }
        if let Some(v) = &self.llm_evidence {
            llm::push_llm_check(checks, limitations, v);
        }
        if let Some(v) = &self.txn_evidence {
            super::txn::push_txn_check(checks, limitations, v);
        }
        if let Some(v) = &self.receipt_evidence {
            super::receipt::push_receipt_check(checks, limitations, v);
        }
        if let Some(v) = &self.subscription_evidence {
            subscription::push_subscription_check(checks, limitations, v);
        }
        if let Some(v) = &self.turn_evidence {
            turn::push_turn_check(checks, limitations, v);
        }
        if let Some(v) = &self.cursor_evidence {
            cursor::push_cursor_check(checks, limitations, v);
        }
        if let Some(v) = &self.session_evidence {
            session::push_session_check(checks, limitations, v);
        }
        if let Some(v) = &self.status_evidence {
            status::push_status_check(checks, limitations, v);
        }
        if let Some(v) = &self.outbox_evidence {
            outbox::push_outbox_check(checks, limitations, v);
        }
        if let Some(v) = &self.session_start_evidence {
            session_start::push_session_start_check(checks, limitations, v);
        }
        if let Some(v) = &self.session_watch_evidence {
            session_watch::push_session_watch_check(checks, limitations, v);
        }
    }

    pub(super) fn event_checked(&self) -> bool {
        self.event_evidence.is_some()
    }

    pub(super) fn txn_checked(&self) -> bool {
        self.txn_evidence.is_some()
    }

    pub(super) fn historical_outbox_store_only(&self) -> bool {
        self.outbox_evidence
            .as_ref()
            .is_some_and(|v| bool_at(v, "store_row_found") && !bool_at(v, "graph_found"))
    }

    pub(super) fn relay_status_without_graph(&self) -> bool {
        self.status_evidence.as_ref().is_some_and(|v| {
            bool_at(v, "relay_status_found")
                && !bool_at(v, "graph_found")
                && !bool_at(v, "session_alive")
        })
    }

    fn event_receipt_surface(&self) -> Option<&str> {
        let surfaces = self
            .event_evidence
            .as_ref()?
            .get("receipt_surfaces")?
            .as_array()?;
        match surfaces.as_slice() {
            [surface] => surface.as_str(),
            _ => None,
        }
    }
}
