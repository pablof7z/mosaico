use super::data::{DebugKind, DebugLine, HookTailSnapshot, SessionPane};
use super::util::*;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn load_hook_tail_snapshot(
    root_filters: &BTreeSet<String>,
    session_filter: &Option<String>,
) -> HookTailSnapshot {
    let mut panes: BTreeMap<String, SessionPane> = BTreeMap::new();
    seed_live_sessions(&mut panes);

    let home = crate::config::mosaico_home();
    let sessions_dir = home.join("sessions");
    let mut unscoped = Vec::new();
    if sessions_dir.is_dir() {
        // New layout: one directory per session under sessions/<id>/
        if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
            for entry in entries.flatten() {
                let dir = entry.path();
                if !dir.is_dir() {
                    continue;
                }
                let dir_name = dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                let hint = if dir_name == "_unscoped" {
                    None
                } else {
                    Some(dir_name.as_str())
                };
                read_session_hook_log(&dir.join("hook-calls.jsonl"), &mut panes, hint);
                let cmd_unscoped =
                    read_session_command_log(&dir.join("command-calls.jsonl"), &mut panes, hint);
                unscoped.extend(cmd_unscoped);
            }
        }
    }
    if let Some(path) = crate::command_forensics::configured_log_path() {
        unscoped.extend(read_configured_command_log(&path, &mut panes));
    }
    enrich_panes_from_store(&mut panes);

    let mut roots = BTreeSet::new();
    let mut sessions = BTreeSet::new();
    for pane in panes.values() {
        if !pane.root.is_empty() {
            roots.insert(pane.root.clone());
        }
        if !pane.session.is_empty() {
            sessions.insert(pane.short.clone());
        }
    }

    let mut panes: Vec<SessionPane> = panes
        .into_values()
        .filter(|p| root_filters.is_empty() || root_filters.contains(&p.root))
        .filter(|p| match session_filter {
            Some(filter) => p.session == *filter || p.short == *filter,
            None => true,
        })
        .collect();
    for pane in &mut panes {
        pane.lines.sort_by_key(|l| l.ts_ms);
        if pane.lines.is_empty() {
            pane.lines.push(DebugLine {
                ts_ms: 0,
                kind: DebugKind::Session,
                label: "session".to_string(),
                summary: "no hook or command telemetry yet".to_string(),
                detail: String::new(),
            });
        }
    }
    panes.sort_by(|a, b| latest_ts(b).cmp(&latest_ts(a)).then(a.short.cmp(&b.short)));

    HookTailSnapshot {
        panes,
        unscoped,
        roots: roots.into_iter().collect(),
        sessions: sessions.into_iter().collect(),
    }
}

fn seed_live_sessions(panes: &mut BTreeMap<String, SessionPane>) {
    let Ok(v) = crate::daemon::blocking::call_no_spawn(
        "who",
        serde_json::json!({
            "workspace": null,
            "all_workspaces": true,
            "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
        }),
    ) else {
        return;
    };
    for row in v["rows"].as_array().cloned().unwrap_or_default() {
        let session = row["pubkey"].as_str().unwrap_or("").to_string();
        if session.is_empty() {
            continue;
        }
        let pane = panes
            .entry(session.clone())
            .or_insert_with(|| new_pane(&session));
        pane.root = non_empty_str(&row["work_root_display"])
            .or_else(|| non_empty_str(&row["work_root"]))
            .or_else(|| non_empty_str(&row["root"]))
            .unwrap_or_default();
        pane.agent = row["slug"].as_str().unwrap_or("").to_string();
        pane.host = row["host"].as_str().unwrap_or("").to_string();
        if let Some(channel) = non_empty_str(&row["channel"]) {
            push_unique(&mut pane.channels, channel);
        }
    }
}

fn enrich_panes_from_store(panes: &mut BTreeMap<String, SessionPane>) {
    enrich_panes_from_store_path(panes, &crate::daemon::store_path());
}

fn enrich_panes_from_store_path(panes: &mut BTreeMap<String, SessionPane>, path: &std::path::Path) {
    let Ok(store) = crate::state::Store::open(path) else {
        return;
    };
    for pane in panes.values_mut() {
        if let Ok(Some(session)) = store.get_session(&pane.session) {
            pane.agent = store
                .handle_for_pubkey(&session.pubkey)
                .ok()
                .flatten()
                .unwrap_or(session.agent_slug);
        }
        enrich_pane_scope_from_store(pane, &store);
    }
}

fn enrich_pane_scope_from_store(pane: &mut SessionPane, store: &crate::state::Store) {
    let Ok(channels) = store.list_session_joined_channels(&pane.session) else {
        return;
    };
    if channels.is_empty() {
        return;
    }
    pane.channels = channels
        .iter()
        .map(|(channel, _)| channel_display_label(store, channel))
        .collect();
    if let Some((channel, _)) = channels.first() {
        let workspace = crate::daemon::workspace_path::WorkspacePathResolver::new(store)
            .root_for_channel(channel);
        pane.root = channel_display_label(store, &workspace);
    }
}

fn channel_display_label(store: &crate::state::Store, channel_h: &str) -> String {
    store
        .get_channel(channel_h)
        .ok()
        .flatten()
        .and_then(|channel| channel.human_name().map(str::to_string))
        .unwrap_or_else(|| channel_h.to_string())
}

fn non_empty_str(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

/// Read a per-session hook log (whole file, no tail limit).
/// `session_hint` is the session_id inferred from the directory name; used as a fallback
/// when stdin doesn't carry a session_id (rare, but possible for early events).
fn read_session_hook_log(
    path: &std::path::Path,
    panes: &mut BTreeMap<String, SessionPane>,
    session_hint: Option<&str>,
) {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return;
    };
    parse_hook_log(&raw, panes, session_hint);
}

fn parse_hook_log(
    raw: &str,
    panes: &mut BTreeMap<String, SessionPane>,
    session_hint: Option<&str>,
) {
    use std::collections::HashMap;
    let mut hook_sessions: HashMap<String, String> = HashMap::new();
    for line in raw.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["schema"].as_str() != Some("mosaico.hook-call.v1") {
            continue;
        }
        let call_id = v["call_id"].as_str().unwrap_or("").to_string();
        let ts_ms = ts_ms(&v);
        match v["phase"].as_str().unwrap_or("") {
            "received" => {
                let host = v["hook"]["host"].as_str().unwrap_or("");
                let hook_type = v["hook"]["type"].as_str().unwrap_or("");
                let stdin_json = &v["stdin"]["json"];
                let session = hook_session(stdin_json)
                    .or_else(|| session_hint.map(str::to_string))
                    .unwrap_or_else(|| "unscoped".to_string());
                hook_sessions.insert(call_id, session.clone());
                let pane = panes
                    .entry(session.clone())
                    .or_insert_with(|| new_pane(&session));
                fill_pane_from_hook(pane, host, stdin_json);
                let (label, summary, detail) = classify_hook(hook_type, stdin_json);
                pane.lines.push(DebugLine {
                    ts_ms,
                    kind: DebugKind::Hook,
                    label,
                    summary,
                    detail,
                });
            }
            "note" => {
                let note = v["note"].as_str().unwrap_or("note");
                let detail_val = &v["detail"];
                let session = detail_val["session"]
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| hook_sessions.get(&call_id).cloned())
                    .or_else(|| session_hint.map(str::to_string))
                    .unwrap_or_else(|| "unscoped".to_string());
                let pane = panes
                    .entry(session.clone())
                    .or_insert_with(|| new_pane(&session));
                if note == "context-audit" {
                    let emitted = detail_val["output"]["emitted"].as_bool().unwrap_or(false);
                    let bytes = detail_val["output"]["bytes"].as_u64().unwrap_or(0);
                    let audit_kind = detail_val["audit"]["kind"].as_str().unwrap_or("context");
                    let summary = if emitted {
                        format!("{audit_kind}: emitted {bytes} bytes")
                    } else {
                        format!("{audit_kind}: no injection")
                    };
                    let detail = serde_json::to_string_pretty(detail_val)
                        .unwrap_or_else(|_| detail_val.to_string());
                    pane.lines.push(DebugLine {
                        ts_ms,
                        kind: DebugKind::Hook,
                        label: "audit".to_string(),
                        summary,
                        detail,
                    });
                } else if note == "context-injection" {
                    let full_text = detail_val["text"].as_str().unwrap_or("").to_string();
                    let summary = full_text
                        .lines()
                        .next()
                        .map(|l| truncate_str(l, 160))
                        .unwrap_or_default();
                    pane.lines.push(DebugLine {
                        ts_ms,
                        kind: DebugKind::Inject,
                        label: "inject".to_string(),
                        summary,
                        detail: full_text,
                    });
                } else {
                    pane.lines.push(DebugLine {
                        ts_ms,
                        kind: DebugKind::Hook,
                        label: note.to_string(),
                        summary: truncate_str(&detail_val.to_string(), 160),
                        detail: detail_val.to_string(),
                    });
                }
            }
            "finished" => {
                let ok = v["result"]["ok"].as_bool();
                // Skip successful completions — they're pure noise.
                if ok != Some(false) {
                    continue;
                }
                let session = hook_sessions
                    .get(&call_id)
                    .cloned()
                    .or_else(|| session_hint.map(str::to_string))
                    .unwrap_or_else(|| "unscoped".to_string());
                let pane = panes
                    .entry(session.clone())
                    .or_insert_with(|| new_pane(&session));
                let error = v["result"]["error"].as_str().unwrap_or("unknown error");
                pane.lines.push(DebugLine {
                    ts_ms,
                    kind: DebugKind::Error,
                    label: "error".to_string(),
                    summary: error.to_string(),
                    detail: error.to_string(),
                });
            }
            _ => {}
        }
    }
}

/// Read a per-session command log. `_unscoped` accumulates across all sessions
/// and can grow to gigabytes, so tail-limit it.
fn read_session_command_log(
    path: &std::path::Path,
    panes: &mut BTreeMap<String, SessionPane>,
    session_hint: Option<&str>,
) -> Vec<DebugLine> {
    let raw = tail_read(path, 2_000_000);
    if raw.is_empty() {
        return Vec::new();
    }
    parse_command_log(&raw, panes, session_hint)
}

/// Read the explicitly configured command log with a byte-limit tail.
fn read_configured_command_log(
    path: &std::path::Path,
    panes: &mut BTreeMap<String, SessionPane>,
) -> Vec<DebugLine> {
    let raw = tail_read(path, 2_000_000);
    if raw.is_empty() {
        return Vec::new();
    }
    parse_command_log(&raw, panes, None)
}

fn parse_command_log(
    raw: &str,
    panes: &mut BTreeMap<String, SessionPane>,
    session_hint: Option<&str>,
) -> Vec<DebugLine> {
    use std::collections::HashMap;
    let mut received: HashMap<String, Value> = HashMap::new();
    let mut unscoped = Vec::new();
    for line in raw.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["schema"].as_str() != Some("mosaico.command-call.v1") {
            continue;
        }
        let call_id = v["call_id"].as_str().unwrap_or("").to_string();
        match v["phase"].as_str().unwrap_or("") {
            "received" => {
                received.insert(call_id, v);
            }
            "finished" => {
                let Some(start) = received.get(&call_id) else {
                    continue;
                };
                let root = command_root(start);
                let agent = start["env"]["MOSAICO_AGENT"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let session = command_session(start)
                    .or_else(|| infer_command_session(panes, &agent, &root))
                    .or_else(|| session_hint.map(str::to_string));
                let argv: Vec<&str> = start["command"]["argv"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();
                // Strip the binary name, show the subcommand
                let subcmd = argv.get(1..).unwrap_or(&[]).join(" ");
                let ok = v["result"]["ok"].as_bool();
                let summary = if ok == Some(false) {
                    let err = v["result"]["error"].as_str().unwrap_or("error");
                    format!("{subcmd}  ✗ {}", truncate_str(err, 80))
                } else {
                    subcmd.clone()
                };
                let detail = if let Some(err) = v["result"]["error"].as_str() {
                    format!("{}\n\nerror: {err}", argv.join(" "))
                } else {
                    argv.join(" ")
                };
                let entry = DebugLine {
                    ts_ms: ts_ms(&v),
                    kind: if ok == Some(false) {
                        DebugKind::Error
                    } else {
                        DebugKind::Command
                    },
                    label: "cmd".to_string(),
                    summary,
                    detail,
                };
                if let Some(session) = session {
                    let pane = panes
                        .entry(session.clone())
                        .or_insert_with(|| new_pane(&session));
                    if !root.is_empty() {
                        pane.root = root;
                    }
                    if !agent.is_empty() {
                        pane.agent = agent;
                    }
                    pane.lines.push(entry);
                } else {
                    unscoped.push(entry);
                }
            }
            _ => {}
        }
    }
    unscoped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RegisterSession;

    #[test]
    fn enriches_pane_agent_with_public_session_handle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let store = crate::state::Store::open(&path).unwrap();
        store
            .reserve_session(&RegisterSession {
                pubkey: "pk".into(),
                observed_harness: "claude-code".into(),
                agent_slug: "haiku".into(),
                channel_h: "aaa".into(),
                child_pid: None,
                transcript_path: None,
                now: 1,
            })
            .unwrap();
        store.bind_session_signer("pk", "test-signer-salt").unwrap();
        store
            .allocate_custom_handle("pk", "haiku", "pearl-cliff-395", 1)
            .unwrap();
        store.upsert_channel("aaa", "aaa", "", "", 1).unwrap();
        store.upsert_channel("dev-h", "dev", "", "aaa", 1).unwrap();
        store.join_session_channel("pk", "dev-h", 2).unwrap();

        let mut panes = BTreeMap::from([(
            "pk".into(),
            SessionPane {
                session: "pk".into(),
                agent: "haiku".into(),
                ..SessionPane::default()
            },
        )]);

        enrich_panes_from_store_path(&mut panes, &path);

        assert_eq!(panes["pk"].agent, "pearl-cliff-395-haiku");
        assert_eq!(panes["pk"].root, "aaa");
        assert_eq!(panes["pk"].channels, vec!["aaa", "dev"]);
    }
}
