use crate::daemon::server::DaemonState;
use anyhow::Result;
use serde_json::{json, Value};
use std::sync::Arc;

pub(super) fn state_value(state: &Arc<DaemonState>) -> Result<Value> {
    let r = state.delivery.lock().expect("delivery mutex poisoned");
    let rows: Vec<Value> = r
        .state_rows()
        .into_iter()
        .map(|row| {
            let session = row.session;
            let resource_key = format!("delivery/{session}");
            json!({
                "session": session,
                "resource_key": resource_key,
                "action": row.action,
                "event_ids": row.event_ids,
                "pty_id": row.pty_id,
                "retry_after_secs": row.retry_after_secs,
            })
        })
        .collect();
    Ok(json!({ "verb": "state", "surface": "delivery", "rows": rows }))
}
