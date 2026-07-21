//! Hook merging, Codex migration, and installation detection.

use super::config::{self, Harness};

const CODEX_ROOT_HOOK_EVENTS: &[&str] =
    &["SessionStart", "UserPromptSubmit", "PostToolUse", "Stop"];

/// Does a hook group contain a mosaico command for `host`?
fn group_is_ours(group: &serde_json::Value, host: &str) -> bool {
    let needle = format!("mosaico harness hook {host} --type ");
    group
        .get("hooks")
        .and_then(|h| h.as_array())
        .map(|hooks| {
            hooks.iter().any(|h| {
                h.get("command")
                    .and_then(|c| c.as_str())
                    .is_some_and(|c| c.contains(&needle))
            })
        })
        .unwrap_or(false)
}

pub(super) fn ensure_object(v: &mut serde_json::Value) {
    if !v.is_object() {
        *v = serde_json::json!({});
    }
}

pub fn ensure_hooks_object(
    root: &mut serde_json::Value,
) -> &mut serde_json::Map<String, serde_json::Value> {
    ensure_object(root);
    let root_obj = root.as_object_mut().expect("root forced to object");
    let hooks = root_obj
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    if !hooks.is_object() {
        *hooks = serde_json::json!({});
    }
    hooks.as_object_mut().expect("hooks forced to object")
}

/// Codex used both root event keys and nested `hooks` JSON during the transition
/// away from TOML. Keep user hooks by moving root event arrays under `hooks`.
pub fn migrate_codex_root_events(root: &mut serde_json::Value) {
    ensure_object(root);
    let Some(root_obj) = root.as_object_mut() else {
        return;
    };
    let mut moved = Vec::new();
    for event in CODEX_ROOT_HOOK_EVENTS {
        if let Some(value) = root_obj.remove(*event) {
            moved.push(((*event).to_string(), value));
        }
    }
    if moved.is_empty() {
        return;
    }

    let hooks = ensure_hooks_object(root);
    for (event, incoming) in moved {
        match (hooks.get_mut(&event), incoming) {
            (Some(serde_json::Value::Array(existing)), serde_json::Value::Array(mut incoming)) => {
                existing.append(&mut incoming);
            }
            (None, value) => {
                hooks.insert(event, value);
            }
            _ => {}
        }
    }
}

/// Merge our hook entries into a `{"hooks": {<Event>: [...]}}` JSON object,
/// replacing any existing groups that match our signature.
pub fn merge_hooks(
    root: &mut serde_json::Value,
    entries: &[(&str, serde_json::Value)],
    host: &str,
    uninstall: bool,
) -> usize {
    let hooks_obj = ensure_hooks_object(root);
    let mut removed = 0usize;
    for (event, entry) in entries {
        let slot = hooks_obj
            .entry((*event).to_string())
            .or_insert_with(|| serde_json::Value::Array(Vec::new()));
        if !slot.is_array() {
            *slot = serde_json::Value::Array(Vec::new());
        }
        let groups = slot.as_array_mut().expect("event forced to array");
        let before = groups.len();
        groups.retain(|g| !group_is_ours(g, host));
        removed += before - groups.len();
        if !uninstall {
            groups.push(entry.clone());
        }
    }
    hooks_obj.retain(|_, v| v.as_array().map(|a| !a.is_empty()).unwrap_or(true));
    removed
}

fn is_json_harness_installed(h: &Harness) -> bool {
    let Ok(content) = std::fs::read_to_string(&h.config_path) else {
        return false;
    };
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    if h.id == "codex" {
        migrate_codex_root_events(&mut v);
    }
    let host = config::host_for_harness(h);
    config::hook_entries(h).iter().all(|(evt, _)| {
        v.get("hooks")
            .and_then(|h| h.get(evt))
            .and_then(|a| a.as_array())
            .is_some_and(|arr| arr.iter().any(|g| group_is_ours(g, host)))
    })
}

fn is_json_harness_present(h: &Harness) -> bool {
    let Ok(content) = std::fs::read_to_string(&h.config_path) else {
        return false;
    };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    if h.id == "codex" {
        migrate_codex_root_events(&mut value);
    }
    let host = config::host_for_harness(h);
    value
        .get("hooks")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|hooks| {
            hooks.values().any(|groups| {
                groups
                    .as_array()
                    .is_some_and(|groups| groups.iter().any(|group| group_is_ours(group, host)))
            })
        })
}

pub fn is_present(h: &Harness) -> bool {
    match h.id {
        "opencode" => h.config_path.exists(),
        "claude-code" | "codex" | "grok" => is_json_harness_present(h),
        "goose" => super::goose::is_present(h),
        "hermes" => super::hermes::is_present(h),
        _ => false,
    }
}

pub fn is_installed(h: &Harness) -> bool {
    match h.id {
        "opencode" => {
            h.config_path.exists()
                && std::fs::read_to_string(&h.config_path)
                    .map(|s| s.contains("mosaico") && s.contains("opencode"))
                    .unwrap_or(false)
        }
        "claude-code" | "codex" | "grok" => is_json_harness_installed(h),
        "goose" => super::goose::is_installed(h),
        "hermes" => super::hermes::is_installed(h),
        _ => false,
    }
}
