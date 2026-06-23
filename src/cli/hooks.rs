use super::turn::{turn_check, turn_end, turn_start, EmitFormat};
use super::*;

mod hook_forensics;

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
    /// Default agent slug (used when neither TENEX_EDGE_AGENT nor
    /// TENEX_EDGE_AGENT_FALLBACK is set).
    agent_slug: &'static str,
    /// JSON fields tried in order to extract the session id from stdin.
    session_id_fields: &'static [&'static str],
    /// Environment variable to check when all session_id_fields are absent or
    /// empty. Used by harnesses (e.g. Grok) that inject the session id via
    /// process environment rather than stdin JSON.
    session_id_env: Option<&'static str>,
    /// JSON field for the live transcript path (None if the harness omits it).
    transcript_field: Option<&'static str>,
    /// Output format for context injection hooks.
    output_format: HookOutputFormat,
    /// Walk process tree for an ancestor whose command contains this string.
    /// None = no watch-pid. Used by harnesses (e.g. Codex) that omit their PID.
    pid_search: Option<&'static str>,
    /// When true, the hook echoes the daemon-minted canonical session id back on
    /// stdout so a programmatic host (e.g. opencode) can adopt it for subsequent
    /// hooks. Such hosts own NO harness-assigned id — the daemon decides identity
    /// from their resume token / tmux pane / watched pid (registered as aliases),
    /// so a missing harness id at session-start is normal, not malformed.
    /// When false (Claude Code, Codex), an empty harness id is a fail-open no-op:
    /// those harnesses always supply their own id, so a missing one means a
    /// malformed payload, and reporting an anchorless observation would mint an
    /// orphan session that later turn-start/stop calls could never match.
    echo_session_id: bool,
}

static HOOK_HOSTS: &[HostDef] = &[
    HostDef {
        name: "claude-code",
        agent_slug: "claude",
        session_id_fields: &["session_id"],
        session_id_env: None,
        transcript_field: Some("transcript_path"),
        output_format: HookOutputFormat::PlainText,
        pid_search: Some("claude"),
        echo_session_id: false,
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
        session_id_env: None,
        transcript_field: Some("transcript_path"),
        output_format: HookOutputFormat::JsonSystemMessage,
        pid_search: Some("codex"),
        echo_session_id: false,
    },
    HostDef {
        // opencode is a programmatic TS plugin, not a stdin-JSON harness in the
        // usual sense: it pipes a small JSON payload to `hook` and reads stdout.
        // It owns no harness-assigned session id, so it no longer mints a
        // competing identity each start: it reports its resume token / pane / PID
        // as locators and the daemon resolves (and reattaches to) the canonical
        // id, which the hook echoes back on stdout. It passes its own PID in the
        // payload (no pid_search).
        name: "opencode",
        agent_slug: "opencode",
        session_id_fields: &["session_id"],
        session_id_env: None,
        transcript_field: Some("transcript_path"),
        output_format: HookOutputFormat::PlainText,
        pid_search: None,
        echo_session_id: true,
    },
    HostDef {
        // Grok Build (xAI) injects the session id via the GROK_SESSION_ID
        // environment variable rather than the JSON payload, so we fall back to
        // that env var when the JSON fields are absent. The workspace root is
        // available as GROK_WORKSPACE_ROOT but current_dir() already points
        // there when the hook is invoked, so no special cwd handling is needed.
        name: "grok",
        agent_slug: "grok",
        session_id_fields: &["session_id", "sessionId"],
        session_id_env: Some("GROK_SESSION_ID"),
        transcript_field: None,
        output_format: HookOutputFormat::PlainText,
        pid_search: Some("grok"),
        echo_session_id: false,
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

    let mut buf = String::new();
    let read_error = std::io::stdin()
        .read_to_string(&mut buf)
        .err()
        .map(|e| e.to_string());
    let parsed = serde_json::from_str::<serde_json::Value>(&buf);
    let parse_error = parsed.as_ref().err().map(|e| e.to_string());
    let raw = parsed.unwrap_or(serde_json::Value::Null);
    let parsed_json = parse_error.is_none().then_some(&raw);
    let call_log = hook_forensics::HookCallLog::start(
        &host_name,
        &hook_type,
        &buf,
        read_error.as_deref(),
        parse_error.as_deref(),
        parsed_json,
    );
    let result = hook_dispatch(host_name, hook_type, raw, &call_log).await;
    call_log.finish(&result);
    result
}

async fn hook_dispatch(
    host_name: String,
    hook_type: String,
    raw: serde_json::Value,
    call_log: &hook_forensics::HookCallLog,
) -> Result<()> {
    let Some(host) = find_hook_host(&host_name) else {
        eprintln!("[tenex-edge] unknown host {host_name:?}; run `--host help` to list");
        call_log.note("unknown-host", serde_json::json!({ "host": host_name }));
        return Ok(());
    };

    // How context is emitted depends on host AND hook type. Claude Code's
    // PostToolUse only reads a `hookSpecificOutput` envelope (plain stdout there
    // is ignored), unlike its UserPromptSubmit which injects plain stdout. Every
    // other (host, hook) pair follows the host's default output format.
    let emit = match (host.name, hook_type.as_str()) {
        ("claude-code", "post-tool-use") => EmitFormat::ClaudePostToolUse,
        _ => match host.output_format {
            HookOutputFormat::PlainText => EmitFormat::PlainText,
            HookOutputFormat::JsonSystemMessage => EmitFormat::JsonSystemMessage,
        },
    };
    let agent_slug = agent_env_slug().unwrap_or_else(|| host.agent_slug.to_string());

    // Parse stdin — fail open if JSON is absent or malformed.
    let obj = raw.as_object();

    let sid: String = host
        .session_id_fields
        .iter()
        .find_map(|f| {
            obj.and_then(|o| o.get(*f))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .or_else(|| {
            host.session_id_env
                .and_then(|k| std::env::var(k).ok().filter(|s| !s.is_empty()))
        })
        .unwrap_or_default();

    let cwd: PathBuf = obj
        .and_then(|o| o.get("cwd"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    // No known project in this directory? Hooks must NOT disturb the agent:
    // exit 0 silently. The user will see the "no known project" message when
    // they run an explicit `tenex-edge` verb from this dir; a harness running
    // here should just proceed without tenex-edge's fabric features.
    if crate::project::resolve(&cwd).is_err() {
        call_log.note(
            "no-project",
            serde_json::json!({ "cwd": cwd.to_string_lossy() }),
        );
        return Ok(());
    }

    let transcript: Option<String> = host.transcript_field.and_then(|field| {
        obj.and_then(|o| o.get(field))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    });

    // Harness-native resume token, forwarded by programmatic hosts whose
    // assigned id differs from our identity (opencode → `ses_*`). claude-code /
    // codex omit it: their adopted `session_id` is already the resume token.
    let resume_id: Option<String> = obj
        .and_then(|o| o.get("resume_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);

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

            // The raw hook id is NOT canonical identity — it is the harness's
            // external session id, one locator among several (resume token, tmux
            // pane, watched pid). We REPORT what we observed; the daemon owns
            // identity and decides whether to mint, reattach, or supersede.
            let harness_session_id = if sid.is_empty() {
                None
            } else {
                Some(sid.clone())
            };

            if harness_session_id.is_none() && !host.echo_session_id {
                // Fail open: a harness that owns its id but dropped it here sent a
                // malformed payload — reporting an anchorless observation would
                // mint an orphan session later hooks could never match.
                call_log.note(
                    "missing-session-id",
                    serde_json::json!({
                        "host": host.name,
                        "hook_type": hook_type,
                        "fields_tried": host.session_id_fields,
                    }),
                );
                return Ok(());
            }

            let canonical = report_observation(
                host,
                &agent_slug,
                &cwd,
                harness_session_id,
                resume_id,
                watch_pid,
            )
            .await?;

            if host.echo_session_id {
                // Programmatic host with no id of its own: hand the daemon-minted
                // canonical id back on stdout so the caller can adopt it for
                // subsequent hooks. Return JSON with both session_id and
                // codename so the caller can display the codename (matching
                // `who` output).
                let codename = crate::util::session_codename(&canonical);
                let json = serde_json::json!({
                    "session_id": canonical,
                    "codename": codename,
                });
                println!("{json}");
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
            // Reassert the session before the turn starts: if the daemon lost it
            // (restart, version-skew kill, crash), this re-registers the live
            // session instead of silently dropping awareness for the whole turn.
            if !sid.is_empty() {
                let watch_pid = obj
                    .and_then(|o| o.get("pid").or_else(|| o.get("watch_pid")))
                    .and_then(|v| v.as_i64())
                    .map(|n| n as i32)
                    .or_else(|| host.pid_search.and_then(find_ancestor_pid));
                // Re-report the observation (not a fresh identity): the daemon
                // resolves the incoming id back to the canonical session via its
                // aliases and re-registers the live session if it was lost.
                if let Err(e) = report_observation(
                    host,
                    &agent_slug,
                    &cwd,
                    Some(sid.clone()),
                    resume_id.clone(),
                    watch_pid,
                )
                .await
                {
                    eprintln!("[tenex-edge] session reassert skipped: {e:#}");
                }
            }
            if let Some(ctx) = turn_start(sid.clone(), transcript, emit).await? {
                call_log.note(
                    "context-injection",
                    serde_json::json!({
                        "host": host.name,
                        "hook_type": hook_type,
                        "session": sid,
                        "bytes": ctx.as_bytes().len(),
                        "text": ctx,
                    }),
                );
            }
            // Publish the user's prompt as kind:9 chat into the session's room
            // (operator-signed; see daemon `rpc_user_prompt`). Fail open: if
            // userNsec is absent or the relay is unreachable, the hook must not
            // block the editor.
            if let Some(prompt_text) = prompt {
                let params = serde_json::json!({
                    "env_session": sid,
                    "agent": agent_env_slug(),
                    "channel": crate::cli::channel_env(),
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
            if let Some(ctx) = turn_check(explicit.clone(), emit)? {
                call_log.note(
                    "context-injection",
                    serde_json::json!({
                        "host": host.name,
                        "hook_type": hook_type,
                        "session": explicit,
                        "bytes": ctx.as_bytes().len(),
                        "text": ctx,
                    }),
                );
            }
        }
        "stop" => {
            if !sid.is_empty() {
                // The agent's turn output (last assistant text) is published as
                // kind:9 chat into the session's room by the daemon. Read it
                // from the transcript; absent/unreadable → no reply mirrored.
                let reply = transcript.as_deref().and_then(|p| {
                    crate::transcript::read_last_assistant_text(std::path::Path::new(p))
                });
                turn_end(sid, reply)?;
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

// ── normalized observation reporting ────────────────────────────────────────

/// Report a NORMALIZED session observation to the daemon and return the
/// canonical session id the daemon resolved for it.
///
/// Hooks no longer decide identity: they describe what they observed (harness,
/// the harness-owned external id if any, the resume token, tmux pane, watched
/// pid, cwd) and the daemon resolves the canonical id — minting a new one,
/// reattaching to an existing one via an alias, or superseding a stale one.
///
/// `harness_session_id` is `Some` only for harnesses that own an id of their own
/// (claude-code, codex); it is `None` for programmatic hosts (opencode) whose
/// only stable anchors are the resume token / tmux pane / watched pid. Each
/// present locator becomes a session alias the registry uses to reattach future
/// starts to the same canonical id.
///
/// `tmux_pane` / `tmux_socket` are read from the hook's environment so the daemon
/// can register a tmux endpoint and (via the pane) reattach a resumed session.
async fn report_observation(
    host: &HostDef,
    agent_slug: &str,
    cwd: &std::path::Path,
    harness_session_id: Option<String>,
    resume_id: Option<String>,
    watch_pid: Option<i32>,
) -> Result<String> {
    let tmux_pane = std::env::var("TMUX_PANE").ok().filter(|s| !s.is_empty());
    let tmux_socket = std::env::var("TMUX").ok().filter(|s| !s.is_empty());
    let params = serde_json::json!({
        "agent": agent_slug,
        "harness": host.name,
        "harness_session_id": harness_session_id,
        "resume_id": resume_id,
        "cwd": cwd.to_string_lossy(),
        "watch_pid": watch_pid,
        "tmux_pane": tmux_pane,
        "tmux_socket": tmux_socket,
        // NIP-29 subgroup id this pane was spawned into (TENEX_EDGE_CHANNEL), when
        // present. The daemon stores the session under this `h` instead of the
        // cwd-derived project so its presence/chat publish into the subgroup.
        "channel": crate::cli::channel_env(),
    });
    let v = super::daemon_call_async_with_items("session_start", params, |item| {
        render_init_progress(&item);
    })
    .await?;
    let sid = v["session_id"]
        .as_str()
        .context("daemon returned no session_id")?
        .to_string();
    Ok(sid)
}

fn render_init_progress(item: &serde_json::Value) {
    if std::env::var("TENEX_EDGE_INIT_PROGRESS").ok().as_deref() == Some("0") {
        return;
    }
    if item.get("kind").and_then(|v| v.as_str()) != Some("init_progress") {
        return;
    }
    let elapsed = item
        .get("elapsed_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or_default();
    let phase = item.get("phase").and_then(|v| v.as_str()).unwrap_or("init");
    let message = item.get("message").and_then(|v| v.as_str()).unwrap_or("");
    eprintln!("[tenex-edge init +{elapsed}ms] {phase}: {message}");
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
