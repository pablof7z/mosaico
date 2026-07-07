//! `probe why <handle>` (§4.3): live causality for one handle, under the
//! reconciler lock, from the dependency-path audit already computed on the last
//! commit. Handles may use command shorthand (`sub:<channel>`,
//! `status:<session>`) or visible Trellis resource paths (`sub/h/<channel>`,
//! `status/<session>`). Everything is rendered through the label registry.
//! When no live audit exists for a handle, that is reported cleanly rather than
//! faked.

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

    if handle.starts_with("sub/") {
        let r = state.subs.lock().expect("subs mutex poisoned");
        let why = r.explain_resource_path(handle).ok_or_else(|| {
            anyhow::anyhow!("probe why: invalid subscription resource key `{handle}`")
        })?;
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

    if let Some(session) = strip_handle_id(handle, &["status:", "status/"]) {
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

    if let Some(session) = strip_handle_id(
        handle,
        &["turn:", "turn/", "turn_lifecycle:", "turn_lifecycle/"],
    ) {
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

    if let Some(session) = strip_handle_id(handle, &["cursor:", "cursor/", "cur:", "cur/"]) {
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

    if let Some(session) =
        strip_handle_id(handle, &["delivery:", "delivery/", "inject:", "inject/"])
    {
        let r = state.delivery.lock().expect("delivery mutex poisoned");
        return Ok(match r.explain_delivery(session) {
            Some(why) => json!({
                "verb": "why",
                "handle": handle,
                "kind": "delivery",
                "resource_key": why.resource_key,
                "last_kind": why.last_kind,
                "cause": why.cause,
                "input_causes": why.input_causes,
                "found": true,
            }),
            None => json!({
                "verb": "why",
                "handle": handle,
                "kind": "delivery",
                "found": false,
                "note": "no command emitted yet on this daemon graph",
            }),
        });
    }

    if let Some(raw) = strip_handle_id(handle, &["outbox:", "outbox/"]) {
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

    if let Some(session) = strip_handle_id(handle, &["session_start:", "session_start/"]) {
        let r = state
            .session_start
            .lock()
            .expect("session_start mutex poisoned");
        return Ok(match r.explain_session_start(session) {
            Some(why) => json!({
                "verb": "why",
                "handle": handle,
                "kind": "session_start",
                "resource_key": why.resource_key,
                "last_kind": why.last_kind,
                "cause": why.cause,
                "input_causes": why.input_causes,
                "found": true,
            }),
            None => json!({
                "verb": "why",
                "handle": handle,
                "kind": "session_start",
                "found": false,
                "note": "no command emitted yet on this daemon graph",
            }),
        });
    }

    if let Some(session) = strip_handle_id(
        handle,
        &[
            "watch:",
            "watch/",
            "session_watch:",
            "session_watch/",
            "session-watch/",
        ],
    ) {
        let r = state
            .session_watch
            .lock()
            .expect("session_watch mutex poisoned");
        return Ok(match r.explain_watch(session) {
            Some(why) => json!({
                "verb": "why",
                "handle": handle,
                "kind": "session_watch",
                "resource_key": why.resource_key,
                "last_kind": why.last_kind,
                "cause": why.cause,
                "input_causes": why.input_causes,
                "found": true,
            }),
            None => json!({
                "verb": "why",
                "handle": handle,
                "kind": "session_watch",
                "found": false,
                "note": "no command emitted yet on this daemon graph",
            }),
        });
    }

    if let Some(session) = strip_handle_id(
        handle,
        &["hook:", "hook/", "hook_context:", "hook_context/"],
    ) {
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
        "probe why: handle must be a probe handle (`sub:<channel>`, `status:<session>`) or visible Trellis resource path (`sub/h/<channel>`, `status/<session>`)"
    ))
}

fn strip_handle_id<'a>(handle: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes.iter().find_map(|prefix| {
        handle.strip_prefix(prefix).and_then(|rest| {
            let id = rest
                .split('/')
                .next()
                .unwrap_or(rest)
                .split('@')
                .next()
                .unwrap_or(rest);
            (!id.is_empty()).then_some(id)
        })
    })
}

#[cfg(test)]
mod tests;
