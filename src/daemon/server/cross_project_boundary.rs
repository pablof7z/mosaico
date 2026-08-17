use super::*;
use crate::cross_project_boundary::{self, FileAccess};
use std::path::Path;

pub(super) fn rpc_classify(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let Some(access) = params
        .get("access")
        .and_then(|value| value.as_str())
        .and_then(parse_access)
    else {
        return Ok(allow());
    };
    let Some(requested_path) = params
        .get("path")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
    else {
        return Ok(allow());
    };
    let Some(cwd) = params
        .get("cwd")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
    else {
        return Ok(allow());
    };
    let Ok(session) = resolve_session_inner(
        state,
        &CallerAnchor::from_params(params),
        ResolveScope::Strict,
    ) else {
        return Ok(allow());
    };
    let Ok(bindings) = state.with_store(|store| {
        crate::daemon::workspace_path::WorkspacePathResolver::new(store).bindings()
    }) else {
        return Ok(allow());
    };
    let Some(notice) = cross_project_boundary::classify(
        state.snapshot().config.cross_project_boundary,
        access,
        &session.work_root,
        Path::new(cwd),
        Path::new(requested_path),
        bindings
            .into_iter()
            .map(|binding| (binding.channel_h, binding.abs_path)),
    ) else {
        return Ok(allow());
    };

    Ok(serde_json::json!({
        "decision": notice.action.as_str(),
        "message": format!(
            "{}: {}",
            if notice.action == crate::config::BoundaryAction::Warn {
                "WARN"
            } else {
                "DENIED"
            },
            notice.message
        ),
        "owner_workspace": notice.owner_workspace,
        "resolved_path": notice.resolved_path.to_string_lossy(),
    }))
}

fn parse_access(value: &str) -> Option<FileAccess> {
    match value {
        "read" => Some(FileAccess::Read),
        "write" => Some(FileAccess::Write),
        _ => None,
    }
}

fn allow() -> serde_json::Value {
    serde_json::json!({"decision": "allow"})
}
