//! Workspace validation for local channel -> filesystem bindings.

use super::report::{bool_at, str_at};
use super::DaemonState;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;

pub(super) fn workspace_target(target: &str) -> Option<&str> {
    target
        .strip_prefix("workspace:")
        .or_else(|| target.strip_prefix("workspace/"))
        .or_else(|| target.strip_prefix("work_root:"))
        .or_else(|| target.strip_prefix("work_root/"))
        .and_then(|rest| rest.split('/').next())
        .filter(|id| !id.trim().is_empty())
}

pub(super) fn workspace_evidence(state: &Arc<DaemonState>, target: &str, requested: &str) -> Value {
    let result = state.with_store(|store| {
        let channel = store.get_channel(requested)?;
        let root_channel = store.root_channel_of(requested)?;
        let direct = store.workspace_binding(requested)?;
        let inherited = root_channel
            .as_deref()
            .filter(|root| *root != requested)
            .map(|root| store.workspace_binding(root))
            .transpose()?
            .flatten();
        Ok::<_, anyhow::Error>((channel, root_channel, direct, inherited))
    });
    let (channel, root_channel, direct, inherited) = match result {
        Ok(v) => v,
        Err(e) => {
            return json!({
                "target": target,
                "channel_h": requested,
                "kind": "workspace",
                "supported": true,
                "found": false,
                "error": e.to_string(),
                "summary": "workspace evidence could not read durable state",
                "reason": e.to_string(),
            });
        }
    };

    let binding = direct.as_ref().or(inherited.as_ref());
    let path = binding.map(|row| row.abs_path.as_str()).unwrap_or("");
    let path_obj = Path::new(path);
    let path_absolute = !path.is_empty() && path_obj.is_absolute();
    let path_exists = path_absolute && path_obj.exists();
    let path_is_dir = path_absolute && path_obj.is_dir();
    let inherited_binding = direct.is_none() && inherited.is_some();
    let channel_found = channel.is_some();
    let found = binding.is_some();
    let ok = found && path_absolute && path_exists && path_is_dir;

    json!({
        "target": target,
        "channel_h": requested,
        "kind": "workspace",
        "supported": true,
        "found": found,
        "channel_found": channel_found,
        "channel_name": channel.as_ref().map(|c| c.name.as_str()).unwrap_or(""),
        "parent": channel.as_ref().map(|c| c.parent.as_str()).unwrap_or(""),
        "root_channel": root_channel.as_deref().unwrap_or(""),
        "direct_binding_found": direct.is_some(),
        "inherited_binding_found": inherited.is_some(),
        "inherited_binding": inherited_binding,
        "binding_channel_h": binding.map(|row| row.channel_h.as_str()).unwrap_or(""),
        "abs_path": path,
        "updated_at": binding.map(|row| row.updated_at).unwrap_or(0),
        "path_absolute": path_absolute,
        "path_exists": path_exists,
        "path_is_dir": path_is_dir,
        "ok": ok,
        "summary": summary(requested, binding, path_absolute, path_exists, path_is_dir),
        "reason": reason(found, channel_found, path_absolute, path_exists, path_is_dir),
    })
}

pub(super) fn push_workspace_check(
    checks: &mut Vec<Value>,
    limitations: &mut Vec<String>,
    evidence: &Value,
) {
    let status = if !str_at(evidence, "error").is_empty()
        || (bool_at(evidence, "found") && !bool_at(evidence, "ok"))
    {
        "failed"
    } else if bool_at(evidence, "ok") {
        "passed"
    } else {
        "not_proven"
    };
    checks.push(json!({
        "name": "workspace",
        "status": status,
        "summary": str_at(evidence, "summary"),
    }));
    if status != "passed" && !str_at(evidence, "reason").is_empty() {
        limitations.push(str_at(evidence, "reason").to_string());
    } else if status == "passed" && !bool_at(evidence, "channel_found") {
        limitations.push(
            "workspace path exists locally, but relay channel metadata is not materialized"
                .to_string(),
        );
    }
}

fn summary(
    requested: &str,
    binding: Option<&crate::state::WorkspaceBinding>,
    path_absolute: bool,
    path_exists: bool,
    path_is_dir: bool,
) -> String {
    let Some(binding) = binding else {
        return format!("channel `{requested}` has no local workspace binding");
    };
    if !path_absolute {
        return format!(
            "channel `{requested}` workspace binding `{}` is not absolute",
            binding.abs_path
        );
    }
    if !path_exists {
        return format!(
            "channel `{requested}` workspace path `{}` does not exist",
            binding.abs_path
        );
    }
    if !path_is_dir {
        return format!(
            "channel `{requested}` workspace path `{}` is not a directory",
            binding.abs_path
        );
    }
    format!(
        "channel `{requested}` workspace path `{}` exists",
        binding.abs_path
    )
}

fn reason(
    found: bool,
    channel_found: bool,
    path_absolute: bool,
    path_exists: bool,
    path_is_dir: bool,
) -> &'static str {
    if !found {
        "no workspace_roots row exists for this channel or its root channel"
    } else if !path_absolute {
        "workspace_roots row stores a non-absolute path"
    } else if !path_exists {
        "workspace path does not exist on this machine"
    } else if !path_is_dir {
        "workspace path exists but is not a directory"
    } else if !channel_found {
        "workspace path exists locally, but relay channel metadata is not materialized"
    } else {
        ""
    }
}
