//! `probe state <surface>` (§4.3): live values for a surface, under its lock.
//! `subscriptions` lists each live REQ with its owner scopes + refcount;
//! `status` lists each session's currently-published content, `turn_lifecycle`
//! lists local turn projections, `cursor` lists high-water decisions,
//! `delivery` lists mention-injection decisions, `session_start` lists advisory staged intents, `session_watch` lists live
//! watched sessions, `outbox` lists publish results, and `hook_context` lists
//! daemon-held per-session graphs.

use super::{required_str, DaemonState};
use anyhow::Result;
use serde_json::{json, Value};
use std::sync::Arc;

mod delivery;

pub(super) fn state_value(state: &Arc<DaemonState>, params: &Value) -> Result<Value> {
    let surface = required_str(params, "surface")?;
    match surface {
        "status" => {
            let r = state.status.lock().expect("status mutex poisoned");
            let rows: Vec<Value> = r
                .state_rows()
                .into_iter()
                .map(|row| {
                    let session = row.session;
                    let resource_key = format!("status/{session}");
                    json!({
                        "session": session,
                        "resource_key": resource_key,
                        "title": row.title,
                        "activity": row.activity,
                        "busy": row.busy,
                        "channels": row.channels,
                    })
                })
                .collect();
            Ok(json!({ "verb": "state", "surface": "status", "rows": rows }))
        }
        "subscriptions" => {
            let r = state.subs.lock().expect("subs mutex poisoned");
            let rows: Vec<Value> = r
                .state_rows()
                .into_iter()
                .map(|row| {
                    json!({
                        "resource_key": row.resource_key,
                        "refcount": row.refcount,
                        "owners": row.owners,
                    })
                })
                .collect();
            Ok(json!({ "verb": "state", "surface": "subscriptions", "rows": rows }))
        }
        "turn_lifecycle" => {
            let r = state
                .turn_lifecycle
                .lock()
                .expect("turn lifecycle mutex poisoned");
            let rows: Vec<Value> = r
                .state_rows()
                .into_iter()
                .map(|row| {
                    let session = row.session;
                    let resource_key = format!("turn_lifecycle/{session}");
                    json!({
                        "session": session,
                        "resource_key": resource_key,
                        "working": row.working,
                        "turn_started_at": row.turn_started_at,
                        "transcript_ref": row.transcript_ref,
                    })
                })
                .collect();
            Ok(json!({ "verb": "state", "surface": "turn_lifecycle", "rows": rows }))
        }
        "cursor" => {
            let r = state.cursor.lock().expect("cursor mutex poisoned");
            let rows: Vec<Value> = r
                .state_rows()
                .into_iter()
                .map(|row| {
                    let session = row.session;
                    let resource_key = format!("cursor/{session}");
                    json!({
                        "session": session,
                        "resource_key": resource_key,
                        "cursor": row.cursor,
                        "last_frame": row.last_frame,
                        "delta_since": row.delta_since,
                    })
                })
                .collect();
            Ok(json!({ "verb": "state", "surface": "cursor", "rows": rows }))
        }
        "delivery" => delivery::state_value(state),
        "session_start" => {
            let r = state
                .session_start
                .lock()
                .expect("session_start mutex poisoned");
            let rows: Vec<Value> = r
                .state_rows()
                .into_iter()
                .map(|row| {
                    let session = row.session_id;
                    let resource_key = format!("session_start/{session}");
                    json!({
                        "session": session,
                        "resource_key": resource_key,
                        "action": row.action,
                        "channel_h": row.channel_h,
                        "signer_pubkey": row.signer_pubkey,
                        "reassert": row.reassert,
                        "failure_stage": row.failure_stage,
                        "failure_error": row.failure_error,
                        "has_channel_ready_intent": row.has_channel_ready_intent,
                        "has_spawn_intent": row.has_spawn_intent,
                        "watch_pid": row.watch_pid,
                        "ensure_subscription": row.ensure_subscription,
                        "replay_chat": row.replay_chat,
                    })
                })
                .collect();
            Ok(json!({ "verb": "state", "surface": "session_start", "rows": rows }))
        }
        "session_watch" => {
            let r = state
                .session_watch
                .lock()
                .expect("session_watch mutex poisoned");
            let rows: Vec<Value> = r
                .state_rows()
                .into_iter()
                .map(|row| {
                    json!({
                        "session": row.session,
                        "resource_key": row.resource_key,
                        "refcount": row.refcount,
                        "owners": row.owners,
                    })
                })
                .collect();
            Ok(json!({ "verb": "state", "surface": "session_watch", "rows": rows }))
        }
        "outbox" => {
            let r = state.outbox.lock().expect("outbox mutex poisoned");
            let rows: Vec<Value> = r
                .state_rows()
                .into_iter()
                .map(|row| {
                    let resource_key = format!("outbox/{}", row.local_id);
                    json!({
                        "local_id": row.local_id,
                        "resource_key": resource_key,
                        "event_id": row.event_id,
                        "state": row.state,
                        "retries": row.retries,
                        "last_error": row.last_error,
                        "source_ref": row.source_ref,
                    })
                })
                .collect();
            Ok(json!({ "verb": "state", "surface": "outbox", "rows": rows }))
        }
        "hook_context" => hook_context_state(state, params),
        other => Err(anyhow::anyhow!("probe state: unknown surface `{other}`")),
    }
}

fn hook_context_state(state: &Arc<DaemonState>, params: &Value) -> Result<Value> {
    let handle = params
        .get("handle")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let dump = params.get("dump").and_then(Value::as_bool).unwrap_or(false);
    let graphs = state
        .hook_contexts
        .lock()
        .expect("hook-context mutex poisoned");

    if let Some(session) = handle {
        let Some(graph) = graphs.get(session) else {
            return Ok(json!({
                "verb": "state",
                "surface": "hook_context",
                "handle": session,
                "found": false,
                "rows": [],
                "note": "no live hook_context graph for session",
            }));
        };
        return Ok(json!({
            "verb": "state",
            "surface": "hook_context",
            "handle": session,
            "found": true,
            "rows": [hook_row(session, graph, dump)],
        }));
    }

    let mut sessions = graphs.keys().cloned().collect::<Vec<_>>();
    sessions.sort();
    let rows = sessions
        .into_iter()
        .filter_map(|session| {
            graphs
                .get(&session)
                .map(|graph| hook_row(&session, graph, dump))
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "verb": "state",
        "surface": "hook_context",
        "found": !rows.is_empty(),
        "rows": rows,
    }))
}

fn hook_row(session: &str, graph: &crate::reconcile::HookContextReconciler, dump: bool) -> Value {
    let view_label = graph.view_label();
    let mut row = json!({
        "session": session,
        "resource_key": view_label
            .clone()
            .unwrap_or_else(|| format!("hook/{session}/view")),
        "revision": graph.revision(),
        "nodes": graph.graph_node_count(),
        "render_count": graph.render_count(),
        "text": graph.current_text(),
        "input_labels": graph.input_labels(),
        "view_label": view_label,
        "why_input_causes": graph.why_view_input_causes(),
    });
    if dump {
        row["debug_dump"] = Value::String(graph.debug_dump());
    }
    row
}

#[cfg(test)]
mod tests {
    use crate::reconcile::{CoverageSnapshot, StatusReconciler, SubscriptionReconciler};
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn status_state_rows_carry_published_content() {
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
            "reading",
            100,
        )
        .unwrap();
        let rows = r.state_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session, "s1");
        assert_eq!(rows[0].activity, "reading");
        assert!(rows[0].busy);
    }

    #[test]
    fn subscription_state_rows_carry_refcounts() {
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
        let rows = r.state_rows();
        // #h + #d for the channel, each owned by daemon + session scope.
        let h = rows
            .iter()
            .find(|r| r.resource_key == "sub/h/general")
            .unwrap();
        assert_eq!(h.refcount, 2);
        assert_eq!(h.owners.len(), 2);
    }
}
