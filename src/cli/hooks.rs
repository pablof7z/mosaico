use super::turn::{turn_check, turn_end, turn_start};
use super::*;

// ── hook adapter registry ─────────────────────────────────────────────────────
//
// Adding a new agent harness: add one entry to HOOK_HOSTS. Zero new code needed
// for harnesses that follow the standard pattern (JSON stdin, plain/JSON stdout).
// Non-standard needs (custom PID detection, exotic output formats) extend the
// HostDef fields rather than adding branches to hook_run.

/// How context blocks are returned to the model by a given harness.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HookOutputFormat {
    /// Plain text on stdout — Claude Code UserPromptSubmit and most harnesses.
    PlainText,
    /// Codex-style JSON: {"systemMessage": "<content>"} — all Codex hook types.
    JsonSystemMessage,
}

struct HostDef {
    /// Canonical harness name used in --host.
    name: &'static str,
    /// Default agent slug (overridden by TENEX_EDGE_AGENT env var).
    agent_slug: &'static str,
    /// JSON fields tried in order to extract the session id from stdin.
    session_id_fields: &'static [&'static str],
    /// JSON field for the live transcript path (None if the harness omits it).
    transcript_field: Option<&'static str>,
    /// Output format for context injection hooks.
    output_format: HookOutputFormat,
    /// Walk process tree for an ancestor whose command contains this string.
    /// None = no watch-pid. Used by harnesses (e.g. Codex) that omit their PID.
    pid_search: Option<&'static str>,
    /// When true, a session-start payload with no session id makes the daemon
    /// GENERATE one and the hook prints it to stdout — for programmatic hosts
    /// (e.g. opencode) that have no harness-assigned id and capture it back.
    /// When false (Claude Code, Codex), an empty session id is a fail-open
    /// no-op: those harnesses always supply their own id, so a missing one means
    /// a malformed payload, and generating would spawn an orphan session that
    /// later turn-start/stop calls could never match.
    generates_sid: bool,
}

static HOOK_HOSTS: &[HostDef] = &[
    HostDef {
        name: "claude-code",
        agent_slug: "claude",
        session_id_fields: &["session_id"],
        transcript_field: Some("transcript_path"),
        output_format: HookOutputFormat::PlainText,
        pid_search: Some("claude"),
        generates_sid: false,
    },
    HostDef {
        name: "codex",
        agent_slug: "codex",
        session_id_fields: &[
            "session_id",
            "sessionId",
            "conversation_id",
            "conversationId",
            "thread_id",
            "threadId",
        ],
        transcript_field: Some("transcript_path"),
        output_format: HookOutputFormat::JsonSystemMessage,
        pid_search: Some("codex"),
        generates_sid: false,
    },
    HostDef {
        // opencode is a programmatic TS plugin, not a stdin-JSON harness in the
        // usual sense: it pipes a small JSON payload to `hook` and reads stdout.
        // It has no harness-assigned session id, so session-start generates one
        // and returns it; it passes its own PID in the payload (no pid_search).
        name: "opencode",
        agent_slug: "opencode",
        session_id_fields: &["session_id"],
        transcript_field: Some("transcript_path"),
        output_format: HookOutputFormat::PlainText,
        pid_search: None,
        generates_sid: true,
    },
];

fn find_hook_host(name: &str) -> Option<&'static HostDef> {
    if name == "help" {
        eprintln!(
            "known hosts: {}",
            HOOK_HOSTS
                .iter()
                .map(|h| h.name)
                .collect::<Vec<_>>()
                .join(", ")
        );
        return None;
    }
    HOOK_HOSTS.iter().find(|h| h.name == name)
}

// ── hook_run ──────────────────────────────────────────────────────────────────

pub(super) async fn hook_run(host_name: String, hook_type: String) -> Result<()> {
    use std::io::Read as _;

    let Some(host) = find_hook_host(&host_name) else {
        eprintln!("[tenex-edge] unknown host {host_name:?}; run `--host help` to list");
        return Ok(());
    };

    let json_out = host.output_format == HookOutputFormat::JsonSystemMessage;
    let agent_slug =
        std::env::var("TENEX_EDGE_AGENT").unwrap_or_else(|_| host.agent_slug.to_string());

    // Parse stdin — fail open if JSON is absent or malformed.
    let raw: serde_json::Value = {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).ok();
        serde_json::from_str(&buf).unwrap_or(serde_json::Value::Null)
    };
    let obj = raw.as_object();

    let sid: String = host
        .session_id_fields
        .iter()
        .find_map(|f| {
            obj.and_then(|o| o.get(*f))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default();

    let cwd: PathBuf = obj
        .and_then(|o| o.get("cwd"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    let transcript: Option<String> = host.transcript_field.and_then(|field| {
        obj.and_then(|o| o.get(field))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    });

    match hook_type.as_str() {
        "session-start" => {
            // PID to watch: an explicit `pid`/`watch_pid` in the payload (set by
            // programmatic hosts like opencode, which know their own process)
            // wins; otherwise walk the process tree for the harness's ancestor.
            let watch_pid = obj
                .and_then(|o| o.get("pid").or_else(|| o.get("watch_pid")))
                .and_then(|v| v.as_i64())
                .map(|n| n as i32)
                .or_else(|| host.pid_search.and_then(find_ancestor_pid));

            if sid.is_empty() {
                if !host.generates_sid {
                    // Fail open: a harness that assigns its own id but dropped it
                    // here sent a malformed payload — don't spawn an orphan.
                    return Ok(());
                }
                // Programmatic host with no id of its own: generate one and hand
                // it back on stdout so the caller can adopt it.
                let new_sid = session_start_inner(agent_slug, None, Some(cwd), watch_pid)?;
                println!("{new_sid}");
            } else {
                // Harness supplied its own id — adopt it, discard the echo.
                session_start_inner(agent_slug, Some(sid), Some(cwd), watch_pid)?;
            }
        }
        "session-end" => {
            if !sid.is_empty() {
                session_end(sid)?;
            }
        }
        "user-prompt-submit" => {
            let prompt: Option<String> = obj
                .and_then(|o| o.get("prompt"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            if !sid.is_empty() {
                let watch_pid = obj
                    .and_then(|o| o.get("pid").or_else(|| o.get("watch_pid")))
                    .and_then(|v| v.as_i64())
                    .map(|n| n as i32)
                    .or_else(|| host.pid_search.and_then(find_ancestor_pid));
                if let Err(e) = session_start_inner(
                    agent_slug.clone(),
                    Some(sid.clone()),
                    Some(cwd.clone()),
                    watch_pid,
                ) {
                    eprintln!("[tenex-edge] session reassert skipped: {e:#}");
                }
            }
            turn_start(sid.clone(), transcript, json_out).await?;
            // Publish the user's prompt as a kind:1 OP on the Nostr fabric.
            // Fail open: if userNsec is absent or the relay is unreachable, the
            // hook must not block the editor.
            if let Some(prompt_text) = prompt {
                let params = serde_json::json!({
                    "env_session": sid,
                    "agent": std::env::var("TENEX_EDGE_AGENT").ok(),
                    "cwd": cwd.to_string_lossy(),
                    "prompt": prompt_text,
                });
                if let Err(e) = daemon_call_async("user_prompt", params).await {
                    eprintln!("[tenex-edge] user_prompt publish skipped: {e:#}");
                }
            }
        }
        "post-tool-use" => {
            let explicit = if sid.is_empty() { None } else { Some(sid) };
            turn_check(explicit, json_out)?;
        }
        "stop" => {
            if !sid.is_empty() {
                turn_end(sid)?;
            }
        }
        other => {
            // Fail open: unknown hook types are ignored so future harness
            // versions can add hooks without breaking this binary.
            eprintln!("[tenex-edge] unrecognised hook type {other:?} for host {host_name}");
        }
    }
    Ok(())
}

// ── process-tree PID search (for harnesses like Codex that omit their PID) ───

/// Walk the process tree upward looking for an ancestor whose command name
/// contains `needle` (case-insensitive). Returns the first match.
fn find_ancestor_pid(needle: &str) -> Option<i32> {
    let needle = needle.to_lowercase();
    let mut pid = std::process::id() as i32;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..16 {
        let ppid = ps_ppid(pid)?;
        if ppid <= 1 || !seen.insert(ppid) {
            return None;
        }
        if ps_comm(ppid).to_lowercase().contains(&needle) {
            return Some(ppid);
        }
        pid = ppid;
    }
    None
}

fn ps_ppid(pid: i32) -> Option<i32> {
    std::process::Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
}

fn ps_comm(pid: i32) -> String {
    std::process::Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}
