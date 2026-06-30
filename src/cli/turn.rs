//! Thin per-turn hook clients (`turn_start` / `turn_check` / `turn_end`) that
//! forward to the daemon over the UDS. The actual context-assembly logic the
//! daemon runs lives in [`context`], kept separate so neither file exceeds the
//! LOC ceiling.

use super::*;

mod context;
pub use context::{assemble_turn_check_context, assemble_turn_start_context};

/// How a context block is emitted to the harness on stdout. Selected per
/// (host, hook-type): plain text is injected directly by Claude Code's
/// UserPromptSubmit and opencode; Codex wraps every hook in `{systemMessage}`;
/// Claude Code's PostToolUse only reads context from a `hookSpecificOutput`
/// envelope (plain stdout there is ignored by the harness).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EmitFormat {
    PlainText,
    JsonSystemMessage,
    ClaudePostToolUse,
}

// ── turn-start / turn-check / turn-end ───────────────────────────────────────

/// `degraded_notice` is a caller-supplied marker (e.g. a failed session reassert)
/// that MUST reach the agent even when the daemon returns no other context — a
/// silently-dropped reassert would leave the turn un-aware with no visible sign.
/// It is prepended to whatever the daemon assembles.
pub(super) async fn turn_start(
    session: String,
    transcript: Option<String>,
    emit: EmitFormat,
    degraded_notice: Option<String>,
) -> Result<Option<String>> {
    if session.is_empty() {
        if let Some(notice) = degraded_notice {
            emit_context(&notice, emit);
            return Ok(Some(notice));
        }
        return Ok(None);
    }
    let params = serde_json::json!({
        "session": session,
        "transcript": transcript,
    });
    // The daemon RPC can itself fail (daemon down/restarting) — exactly the case a
    // degradation marker exists for. If we have one, don't `?`-return and drop it:
    // log the RPC error loudly and still surface the notice so the agent sees the
    // "⚠ Fabric temporarily unavailable" marker instead of a silent un-aware turn.
    let v = match super::daemon_call_hook_async("turn_start", params).await {
        Ok(v) => v,
        Err(e) => {
            if let Some(notice) = degraded_notice {
                tracing::error!(error = %format!("{e:#}"), "turn_start: daemon RPC failed; emitting degraded marker only");
                emit_context(&notice, emit);
                return Ok(Some(notice));
            }
            return Err(e);
        }
    };
    let combined = match (degraded_notice.as_deref(), v["context"].as_str()) {
        (Some(n), Some(c)) => Some(format!("{n}\n\n{c}")),
        (Some(n), None) => Some(n.to_string()),
        (None, Some(c)) => Some(c.to_string()),
        (None, None) => None,
    };
    if let Some(ctx) = combined {
        emit_context(&ctx, emit);
        return Ok(Some(ctx));
    }
    Ok(None)
}

/// Mid-turn check for PostToolUse hooks. Thin client: the daemon peeks the
/// inbox and computes the rate-limited sibling-session delta.
pub(super) fn turn_check(session: Option<String>, emit: EmitFormat) -> Result<Option<String>> {
    if crate::daemon::is_inhibited() {
        return Ok(None);
    }
    let params = serde_json::json!({
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": agent_env_slug(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
    });
    let v = crate::daemon::blocking::call("turn_check", params)?;
    if let Some(ctx) = v["context"].as_str() {
        emit_context(ctx, emit);
        return Ok(Some(ctx.to_string()));
    }
    Ok(None)
}

fn emit_context(content: &str, emit: EmitFormat) {
    match emit {
        EmitFormat::PlainText => println!("{content}"),
        EmitFormat::JsonSystemMessage => {
            let obj = serde_json::json!({ "systemMessage": content });
            println!("{obj}");
        }
        EmitFormat::ClaudePostToolUse => {
            // Claude Code only reads PostToolUse context from this envelope;
            // plain stdout there is ignored by the harness.
            let obj = serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PostToolUse",
                    "additionalContext": content,
                }
            });
            println!("{obj}");
        }
    }
}

pub(super) fn turn_end(session: String, reply: Option<String>) -> Result<()> {
    if session.is_empty() || crate::daemon::is_inhibited() {
        return Ok(());
    }
    crate::daemon::blocking::call(
        "turn_end",
        serde_json::json!({"session": session, "reply": reply}),
    )?;
    Ok(())
}
