use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const LOG_SCHEMA: &str = "tenex-edge.hook-call.v1";

pub(super) struct HookCallLog {
    path: Option<PathBuf>,
    call_id: String,
}

impl HookCallLog {
    pub(super) fn start(
        host: &str,
        hook_type: &str,
        stdin: &str,
        read_error: Option<&str>,
        parse_error: Option<&str>,
        parsed_json: Option<&Value>,
    ) -> Self {
        let path = log_path(parsed_json);
        let call_id = call_id();
        let payload = serde_json::json!({
            "schema": LOG_SCHEMA,
            "phase": "received",
            "call_id": call_id,
            "timestamp": timestamp(),
            "hook": {
                "host": host,
                "type": hook_type,
            },
            "process": process_snapshot(),
            "parent_chain": parent_chain(),
            "stdin": {
                "bytes": stdin.len(),
                "is_empty": stdin.is_empty(),
                "read_error": read_error,
                "parse_error": parse_error,
                "raw": stdin,
                "json": parsed_json,
                "json_object_keys": parsed_json
                    .and_then(|v| v.as_object())
                    .map(|o| o.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default(),
            },
            "env": redacted_env(),
        });
        append_json(path.as_ref(), &payload);
        Self { path, call_id }
    }

    pub(super) fn note(&self, note: &str, detail: Value) {
        let payload = serde_json::json!({
            "schema": LOG_SCHEMA,
            "phase": "note",
            "call_id": self.call_id,
            "timestamp": timestamp(),
            "note": note,
            "detail": detail,
        });
        append_json(self.path.as_ref(), &payload);
    }

    pub(super) fn context_audit(
        &self,
        host: &str,
        hook_type: &str,
        session: Option<&str>,
        audit: Value,
        context: Option<&str>,
    ) {
        self.note(
            "context-audit",
            serde_json::json!({
                "host": host,
                "hook_type": hook_type,
                "session": session,
                "output": output_json(context),
                "audit": audit,
            }),
        );
        if let Some(text) = context {
            self.note(
                "context-injection",
                serde_json::json!({
                    "host": host,
                    "hook_type": hook_type,
                    "session": session,
                    "bytes": text.len(),
                    "text": text,
                }),
            );
        }
    }

    pub(super) fn finish(&self, result: &Result<()>) {
        let payload = serde_json::json!({
            "schema": LOG_SCHEMA,
            "phase": "finished",
            "call_id": self.call_id,
            "timestamp": timestamp(),
            "result": match result {
                Ok(()) => serde_json::json!({ "ok": true }),
                Err(e) => serde_json::json!({ "ok": false, "error": format!("{e:#}") }),
            },
        });
        append_json(self.path.as_ref(), &payload);
    }
}

fn log_path(parsed_json: Option<&Value>) -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("TENEX_EDGE_HOOK_CALL_LOG") {
        let trimmed = raw.trim();
        if matches!(trimmed, "" | "0" | "false" | "off" | "none") {
            return None;
        }
        return Some(PathBuf::from(trimmed));
    }
    let session_id = parsed_json.and_then(|v| {
        [
            "session_id",
            "sessionId",
            "conversation_id",
            "conversationId",
            "thread_id",
            "threadId",
        ]
        .iter()
        .find_map(|k| v[*k].as_str())
        .filter(|s| !s.is_empty())
    });
    let dir = match session_id {
        Some(id) => crate::config::edge_home().join("sessions").join(id),
        None => crate::config::edge_home()
            .join("sessions")
            .join("_unscoped"),
    };
    Some(dir.join("hook-calls.jsonl"))
}

fn append_json(path: Option<&PathBuf>, payload: &Value) {
    let Some(path) = path else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = serde_json::to_writer(&mut file, payload);
        let _ = writeln!(file);
    }
}

fn output_json(context: Option<&str>) -> Value {
    serde_json::json!({
        "emitted": context.is_some(),
        "bytes": context.map(str::len).unwrap_or(0),
        "text": context,
    })
}

fn call_id() -> String {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "{}.{:09}-{}",
        d.as_secs(),
        d.subsec_nanos(),
        std::process::id()
    )
}

fn timestamp() -> Value {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    serde_json::json!({
        "unix_ms": d.as_millis(),
        "utc": format_utc(d.as_secs()),
    })
}

fn format_utc(unix_secs: u64) -> String {
    unsafe {
        let t = unix_secs as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::gmtime_r(&t, &mut tm).is_null() {
            return unix_secs.to_string();
        }
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec,
        )
    }
}

fn process_snapshot() -> Value {
    serde_json::json!({
        "pid": std::process::id(),
        "ppid": ps_ppid(std::process::id() as i32),
        "exe": std::env::current_exe().ok().map(|p| p.display().to_string()),
        "argv": std::env::args().collect::<Vec<_>>(),
        "cwd": std::env::current_dir().ok().map(|p| p.display().to_string()),
    })
}

fn parent_chain() -> Vec<Value> {
    let mut out = Vec::new();
    let mut pid = std::process::id() as i32;
    for _ in 0..12 {
        let Some(ppid) = ps_ppid(pid) else {
            break;
        };
        if ppid <= 1 {
            break;
        }
        out.push(serde_json::json!({
            "pid": ppid,
            "ppid": ps_ppid(ppid),
            "comm": ps_field(ppid, "comm="),
            "args": ps_field(ppid, "args="),
        }));
        pid = ppid;
    }
    out
}

fn ps_ppid(pid: i32) -> Option<i32> {
    ps_field(pid, "ppid=")?.trim().parse().ok()
}

fn ps_field(pid: i32, field: &str) -> Option<String> {
    std::process::Command::new("ps")
        .args(["-o", field, "-p", &pid.to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn redacted_env() -> BTreeMap<String, String> {
    std::env::vars()
        .map(|(key, value)| {
            let value = if is_sensitive_env(&key) {
                format!("<redacted:{} bytes>", value.len())
            } else {
                value
            };
            (key, value)
        })
        .collect()
}

fn is_sensitive_env(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("api_key")
        || key.contains("apikey")
        || key.contains("token")
        || key.contains("secret")
        || key.contains("password")
        || key.contains("passwd")
        || key.contains("credential")
        || key.contains("private")
        || key.contains("bearer")
        || key.contains("nsec")
        || (key.contains("auth") && key != "ssh_auth_sock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_audit_logs_emitted_and_empty_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hook-calls.jsonl");
        let log = HookCallLog {
            path: Some(path.clone()),
            call_id: "call-1".to_string(),
        };

        log.context_audit(
            "claude-code",
            "post-tool-use",
            Some("sess-1"),
            serde_json::json!({
                "kind": "turn_check",
                "delta_gate": { "cursor_before": 10, "cursor_after": 11 },
            }),
            None,
        );
        log.context_audit(
            "claude-code",
            "user-prompt-submit",
            Some("sess-1"),
            serde_json::json!({ "kind": "turn_start" }),
            Some("<tenex-edge>"),
        );

        let raw = std::fs::read_to_string(path).unwrap();
        let rows = raw
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(rows[0]["note"], "context-audit");
        assert_eq!(rows[0]["detail"]["output"]["emitted"], false);
        assert_eq!(rows[0]["detail"]["audit"]["kind"], "turn_check");
        assert_eq!(rows[1]["note"], "context-audit");
        assert_eq!(rows[1]["detail"]["output"]["text"], "<tenex-edge>");
        assert_eq!(rows[2]["note"], "context-injection");
    }
}
