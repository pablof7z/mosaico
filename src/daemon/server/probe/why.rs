//! `probe why <handle>` (§4.3): live causality for one handle, under the
//! reconciler lock, from the dependency-path audit already computed on the last
//! commit. Handles: `sub:<channel>`, `status:<session>`, `turn:<session>`,
//! `cursor:<session>`, `outbox:<local_id>`, and `hook:<session>`. Everything is
//! rendered through the label registry. When no live audit exists for a handle,
//! that is reported cleanly rather than faked.

use super::{required_str, DaemonState};
use anyhow::Result;
use serde_json::{json, Value};
use std::sync::Arc;

pub(super) fn why_value(state: &Arc<DaemonState>, params: &Value) -> Result<Value> {
    let handle = required_str(params, "handle")?;

    if let Some(channel) = handle.strip_prefix("sub:") {
        let r = state.subs.lock().expect("subs mutex poisoned");
        let why = r.explain_channel(channel);
        return Ok(json!({
            "verb": "why",
            "handle": handle,
            "kind": "subscription",
            "resource_key": why.resource_key,
            "refcount": why.refcount,
            "owners": why.owners,
            "last_kind": why.last_kind,
            "cause": why.cause,
            "input_causes": why.input_causes,
            "found": why.last_kind.is_some(),
        }));
    }

    if let Some(session) = handle.strip_prefix("status:") {
        let r = state.status.lock().expect("status mutex poisoned");
        return Ok(match r.explain_status(session) {
            Some(why) => json!({
                "verb": "why",
                "handle": handle,
                "kind": "status",
                "resource_key": why.resource_key,
                "last_kind": why.last_kind,
                "cause": why.cause,
                "input_causes": why.input_causes,
                "found": true,
            }),
            None => json!({
                "verb": "why",
                "handle": handle,
                "kind": "status",
                "found": false,
                "note": "no command emitted yet on this daemon graph",
            }),
        });
    }

    if let Some(session) = handle
        .strip_prefix("turn:")
        .or_else(|| handle.strip_prefix("turn_lifecycle:"))
    {
        let r = state
            .turn_lifecycle
            .lock()
            .expect("turn lifecycle mutex poisoned");
        return Ok(match r.explain_turn(session) {
            Some(why) => json!({
                "verb": "why",
                "handle": handle,
                "kind": "turn_lifecycle",
                "resource_key": why.resource_key,
                "last_kind": why.last_kind,
                "cause": why.cause,
                "input_causes": why.input_causes,
                "found": true,
            }),
            None => json!({
                "verb": "why",
                "handle": handle,
                "kind": "turn_lifecycle",
                "found": false,
                "note": "no command emitted yet on this daemon graph",
            }),
        });
    }

    if let Some(session) = handle
        .strip_prefix("cursor:")
        .or_else(|| handle.strip_prefix("cur:"))
    {
        let r = state.cursor.lock().expect("cursor mutex poisoned");
        return Ok(match r.explain_cursor(session) {
            Some(why) => json!({
                "verb": "why",
                "handle": handle,
                "kind": "cursor",
                "resource_key": why.resource_key,
                "last_kind": why.last_kind,
                "cause": why.cause,
                "input_causes": why.input_causes,
                "found": true,
            }),
            None => json!({
                "verb": "why",
                "handle": handle,
                "kind": "cursor",
                "found": false,
                "note": "no command emitted yet on this daemon graph",
            }),
        });
    }

    if let Some(raw) = handle.strip_prefix("outbox:") {
        let local_id = raw
            .parse::<i64>()
            .map_err(|_| anyhow::anyhow!("probe why: invalid outbox local id `{raw}`"))?;
        let r = state.outbox.lock().expect("outbox mutex poisoned");
        return Ok(match r.explain_outbox(local_id) {
            Some(why) => json!({
                "verb": "why",
                "handle": handle,
                "kind": "outbox",
                "resource_key": why.resource_key,
                "last_kind": why.last_kind,
                "cause": why.cause,
                "input_causes": why.input_causes,
                "found": true,
            }),
            None => json!({
                "verb": "why",
                "handle": handle,
                "kind": "outbox",
                "found": false,
                "note": "no command emitted yet on this daemon graph",
            }),
        });
    }

    if let Some(session) = handle
        .strip_prefix("hook:")
        .or_else(|| handle.strip_prefix("hook_context:"))
    {
        let r = state
            .hook_contexts
            .lock()
            .expect("hook-context mutex poisoned");
        return Ok(match r.get(session) {
            Some(graph) => json!({
                "verb": "why",
                "handle": handle,
                "kind": "hook_context",
                "resource_key": format!("hook/{session}/view"),
                "last_kind": "View",
                "cause": "dependency-path audit",
                "input_causes": graph.why_view_input_causes(),
                "found": true,
            }),
            None => json!({
                "verb": "why",
                "handle": handle,
                "kind": "hook_context",
                "found": false,
                "note": "no live hook_context graph for session",
            }),
        });
    }

    Err(anyhow::anyhow!(
        "probe why: handle must be `sub:<channel>`, `status:<session>`, `turn:<session>`, `cursor:<session>`, `outbox:<local_id>`, or `hook:<session>`"
    ))
}

#[cfg(test)]
mod tests {
    use crate::reconcile::{
        CoverageSnapshot, StatusReconciler, SubscriptionReconciler, TurnLifecycleReconciler,
        TurnProjectionSeed,
    };
    use std::collections::{BTreeMap, BTreeSet};

    /// `status:` explain surfaces the labeled last command + its input cause.
    #[test]
    fn status_handle_explains_last_command() {
        let mut r = StatusReconciler::new(90, 30);
        r.on_session_started(
            "s1",
            "laptop",
            "coder",
            "pk1",
            ".",
            BTreeSet::from(["room".to_string()]),
            true,
            "T",
            "",
            100,
        )
        .unwrap();
        r.on_distill("s1", "T", "compiling", 100).unwrap();

        let why = r.explain_status("s1").unwrap();
        assert_eq!(why.resource_key, "status/s1");
        assert_eq!(why.last_kind, "Replace");
        assert!(why.input_causes.iter().any(|l| l == "status/s1/activity"));
    }

    /// `sub:` explain surfaces owners + refcount + the labeled cause.
    #[test]
    fn sub_handle_explains_owners_and_refcount() {
        let mut r = SubscriptionReconciler::new().unwrap();
        let mut sessions = BTreeMap::new();
        sessions.insert("s1".to_string(), BTreeSet::from(["general".to_string()]));
        r.sync(&CoverageSnapshot {
            daemon_channels: BTreeSet::from(["general".to_string()]),
            addressed_pubkeys: BTreeSet::new(),
            archived_channels: BTreeSet::new(),
            sessions,
        })
        .unwrap();

        let why = r.explain_channel("general");
        assert_eq!(why.resource_key, "sub/h/general");
        assert_eq!(why.refcount, 2);
        assert_eq!(why.last_kind.as_deref(), Some("Open"));
    }

    #[test]
    fn turn_handle_explains_projection_cause() {
        let mut r = TurnLifecycleReconciler::new();
        r.on_turn_started(
            TurnProjectionSeed {
                session_id: "s1".into(),
                working: false,
                turn_started_at: 0,
                transcript_ref: None,
            },
            100,
            None,
        )
        .unwrap();

        let why = r.explain_turn("s1").unwrap();
        assert_eq!(why.resource_key, "turn_lifecycle/s1");
        assert_eq!(why.last_kind, "Open");
        assert!(why
            .input_causes
            .iter()
            .any(|l| l == "turn_lifecycle/s1/turn_started"));
    }

    #[test]
    fn cursor_handle_explains_projection_cause() {
        let mut r = crate::reconcile::CursorReconciler::new();
        r.request(
            crate::reconcile::CursorSeed {
                session_id: "s1".into(),
                seen_cursor: 10,
            },
            crate::reconcile::InputFact::TurnCheckRequested {
                session_id: "s1".into(),
                observed_cursor: 10,
                working: true,
                at: 20,
            },
        )
        .unwrap();

        let why = r.explain_cursor("s1").unwrap();
        assert_eq!(why.resource_key, "cursor/s1");
        assert!(why
            .input_causes
            .iter()
            .any(|l| l == "cursor/s1/observed_cursor"));
    }

    #[test]
    fn outbox_handle_explains_projection_cause() {
        let mut r = crate::reconcile::OutboxReconciler::new();
        r.drive(crate::reconcile::InputFact::OutboxEnqueueApplied {
            local_id: 7,
            event_id: "ev7".into(),
            event_hash: "sha256:event".into(),
            source_surface: "status".into(),
            source_ref: "status/s1#tx:1".into(),
            at: 100,
        })
        .unwrap();

        let why = r.explain_outbox(7).unwrap();
        assert_eq!(why.resource_key, "outbox/7");
        assert!(why.input_causes.iter().any(|l| l == "outbox/7/event_id"));
    }
}
