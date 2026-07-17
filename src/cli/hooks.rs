use super::turn::{turn_check, turn_end, turn_start, EmitFormat};
use super::*;
use std::path::PathBuf;

mod applicability;
mod class_b;
mod hook_forensics;
mod observation;

use observation::{find_ancestor_pid, find_direct_agent_invocation, report_observation};

// ── hook adapter registry ─────────────────────────────────────────────────────
//
// Standard harnesses add one HOOK_HOSTS entry; non-standard needs extend
// HostDef fields rather than adding branches to hook_run.

/// How context blocks are returned to the model by a given harness.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HookOutputFormat {
    /// Plain text on stdout — Claude Code UserPromptSubmit and most harnesses.
    PlainText,
    /// Codex reads model-visible hook context from event-specific JSON output.
    HookSpecificAdditionalContext,
}

pub(super) struct HostDef {
    /// Canonical harness name used in --host.
    pub(super) name: &'static str,
    /// Default agent slug (used when `MOSAICO_AGENT` is not set).
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
    /// Whether this harness must supply its own native session locator. When true,
    /// an empty locator is a fail-open no-op:
    /// those harnesses always supply their own id, so a missing one means a
    /// malformed payload, and reporting an anchorless observation would mint an
    /// orphan session that later turn-start/stop calls could never match.
    requires_harness_session: bool,
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
        requires_harness_session: true,
    },
    HostDef {
        name: "codex",
        agent_slug: "codex",
        session_id_fields: &["session_id"],
        session_id_env: None,
        transcript_field: Some("transcript_path"),
        output_format: HookOutputFormat::HookSpecificAdditionalContext,
        pid_search: Some("codex"),
        requires_harness_session: true,
    },
    HostDef {
        // opencode is a programmatic TS plugin, not a stdin-JSON harness in the
        // usual sense: it pipes a small JSON payload to `hook` and reads stdout.
        // Its initial startup has no session locator yet, so watch PID is enough.
        // Subsequent lifecycle hooks carry opencode's own native session id.
        name: "opencode",
        agent_slug: "opencode",
        session_id_fields: &["session_id"],
        session_id_env: None,
        transcript_field: Some("transcript_path"),
        output_format: HookOutputFormat::PlainText,
        pid_search: None,
        requires_harness_session: false,
    },
    HostDef {
        // Grok Build (xAI) injects the session id via the GROK_SESSION_ID
        // environment variable rather than the JSON payload, so we fall back to
        // that env var when the JSON fields are absent. The workspace root is
        // available as GROK_WORKSPACE_ROOT but current_dir() already points
        // there when the hook is invoked, so no special cwd handling is needed.
        name: "grok",
        agent_slug: "grok",
        session_id_fields: &["session_id"],
        session_id_env: Some("GROK_SESSION_ID"),
        transcript_field: None,
        output_format: HookOutputFormat::PlainText,
        pid_search: Some("grok"),
        requires_harness_session: true,
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

pub(super) fn caller_watch_pid_anchor() -> Option<(&'static str, i32)> {
    HOOK_HOSTS
        .iter()
        .filter_map(|host| host.pid_search.map(|needle| (host.name, needle)))
        .find_map(|(name, needle)| find_ancestor_pid(needle).map(|pid| (name, pid)))
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
        eprintln!("[mosaico] unknown host {host_name:?}; run `--host help` to list");
        call_log.note("unknown-host", serde_json::json!({ "host": host_name }));
        return Ok(());
    };

    // How context is emitted depends on host AND hook type. Claude Code's
    // PostToolUse and Codex turn hooks read model-visible context from the
    // `hookSpecificOutput.additionalContext` envelope.
    let emit = match (host.name, hook_type.as_str()) {
        ("claude-code", "post-tool-use") => EmitFormat::HookSpecificAdditionalContext {
            hook_event_name: "PostToolUse",
        },
        _ => match host.output_format {
            HookOutputFormat::PlainText => EmitFormat::PlainText,
            HookOutputFormat::HookSpecificAdditionalContext => {
                EmitFormat::HookSpecificAdditionalContext {
                    hook_event_name: hook_event_name(&hook_type),
                }
            }
        },
    };
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

    if let Some((note, detail)) = applicability::inapplicable(&cwd) {
        call_log.note(note, detail);
        return Ok(());
    }

    // A slug from MOSAICO_AGENT (set by `mosaico agents`) is always
    // authoritative. Otherwise, look for a live ancestor directly running
    // `claude --agent <name>` (bypassing `mosaico agents`) and treat it the
    // same as if it had been launched under that identity. The profile name is
    // retained, but argv remains owned by harnesses.json.
    let env_slug = agent_env_slug();
    let (agent_slug, profile): (String, Option<String>) = match &env_slug {
        Some(s) => (s.clone(), None),
        None if host.name == "claude-code" => find_direct_agent_invocation()
            .map(|slug| (slug.clone(), Some(slug)))
            .unwrap_or_else(|| (host.agent_slug.to_string(), None)),
        None => (host.agent_slug.to_string(), None),
    };

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

    // Turn hooks resolve identity from `sid`; an empty `sid` skips fabric
    // injection entirely. When the harness DID send a session-id field but it
    // resolved empty (malformed payload), that is a silent awareness drop for
    // the whole turn — emit a loud forensic note mirroring session-start's
    // `missing-session-id`, rather than a quiet no-op. (session-start has its own
    // dedicated handling below, so it is excluded here.)
    if sid.is_empty()
        && matches!(hook_type.as_str(), "user-prompt-submit" | "post-tool-use")
        && obj
            .map(|o| host.session_id_fields.iter().any(|f| o.contains_key(*f)))
            .unwrap_or(false)
    {
        eprintln!(
            "[mosaico] session-id field(s) present but empty for host {} ({:?}); \
             fabric injection skipped this turn",
            host.name, host.session_id_fields
        );
        call_log.note(
            "empty-session-id",
            serde_json::json!({
                "host": host.name,
                "hook_type": hook_type,
                "fields_tried": host.session_id_fields,
            }),
        );
    }

    match hook_type.as_str() {
        "session-start" => {
            let has_pty_anchor = std::env::var("MOSAICO_PTY_SESSION")
                .ok()
                .filter(|s| !s.is_empty())
                .is_some();
            // PID to watch: an explicit `pid`/`watch_pid` in the payload (set by
            // programmatic hosts like opencode, which know their own process)
            // wins; otherwise walk the process tree for the harness's ancestor.
            let watch_pid = obj
                .and_then(|o| o.get("pid").or_else(|| o.get("watch_pid")))
                .and_then(|v| v.as_i64())
                .map(|n| n as i32)
                .or_else(|| host.pid_search.and_then(find_ancestor_pid));

            // The raw hook id is NOT canonical identity — it is the harness's
            // external session id, one locator among several (resume token, hosted
            // PTY, watched pid). We REPORT what we observed; the daemon owns
            // identity and decides whether to mint, reattach, or supersede.
            let harness_session_id = if sid.is_empty() {
                None
            } else {
                Some(sid.clone())
            };

            if harness_session_id.is_none() && host.requires_harness_session && !has_pty_anchor {
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

            if let Err(e) = report_observation(
                host,
                &agent_slug,
                &cwd,
                harness_session_id,
                resume_id,
                watch_pid,
                profile.clone(),
            )
            .await
            {
                eprintln!("[mosaico] session-start hook skipped: {e:#}");
                return Ok(());
            }
        }
        "session-end" => {
            if !sid.is_empty() {
                session_end_hook(sid)?;
            }
        }
        "user-prompt-submit" => {
            // Reassert the session before the turn starts: if the daemon lost it
            // (restart, version-skew kill, crash), this re-registers the live
            // session instead of silently dropping awareness for the whole turn.
            let mut degraded_notice: Option<String> = None;
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
                    profile,
                )
                .await
                {
                    eprintln!("[mosaico] session reassert skipped: {e:#}");
                    // Don't silently drop awareness for the turn: hand the turn a
                    // visible degradation marker so the agent knows the fabric was
                    // temporarily unavailable rather than assuming a quiet channel.
                    degraded_notice = Some(
                        "<mosaico>\n⚠ Fabric temporarily unavailable — this session could not be \
                         reasserted with the daemon, so your inbox and channel awareness for this \
                         turn may be incomplete. Do NOT assume the channel is quiet or that you have \
                         no mentions.\n</mosaico>"
                            .to_string(),
                    );
                }
            }
            let result = turn_start(sid.clone(), transcript, emit, degraded_notice).await?;
            call_log.context_audit(
                host.name,
                &hook_type,
                Some(&sid),
                result.audit,
                result.context.as_deref(),
            );
        }
        "post-tool-use" => {
            let explicit = if sid.is_empty() { None } else { Some(sid) };
            let result = turn_check(explicit.clone(), emit).await?;
            call_log.context_audit(
                host.name,
                &hook_type,
                explicit.as_deref(),
                result.audit,
                result.context.as_deref(),
            );
        }
        "stop" => {
            if !sid.is_empty() {
                turn_end(sid).await?;
                class_b::inject_idle_nudge(&agent_slug);
            }
        }
        other => {
            // Fail open: unknown hook types are ignored so future harness
            // versions can add hooks without breaking this binary.
            eprintln!("[mosaico] unrecognised hook type {other:?} for host {host_name}");
        }
    }
    Ok(())
}

fn hook_event_name(hook_type: &str) -> &'static str {
    match hook_type {
        "session-start" => "SessionStart",
        "session-end" => "SessionEnd",
        "user-prompt-submit" => "UserPromptSubmit",
        "post-tool-use" => "PostToolUse",
        "stop" => "Stop",
        _ => "Unknown",
    }
}
