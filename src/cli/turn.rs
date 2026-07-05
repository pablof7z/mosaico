//! Thin per-turn hook clients (`turn_start` / `turn_check` / `turn_end`) that
//! forward to the daemon over the UDS. The actual context-assembly logic the
//! daemon runs lives in [`context`], kept separate so neither file exceeds the
//! LOC ceiling.

use super::*;

/// How a context block is emitted to the harness on stdout. Selected per
/// (host, hook-type): plain text is injected directly by Claude Code's
/// UserPromptSubmit and opencode; Codex and Claude Code PostToolUse use a
/// `hookSpecificOutput.additionalContext` envelope for model-visible context.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EmitFormat {
    PlainText,
    HookSpecificAdditionalContext { hook_event_name: &'static str },
}

pub(super) struct HookContextResult {
    pub(super) context: Option<String>,
    pub(super) audit: serde_json::Value,
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
) -> Result<HookContextResult> {
    if session.is_empty() {
        if let Some(notice) = degraded_notice {
            emit_context(&notice, emit);
            return Ok(HookContextResult {
                context: Some(notice.clone()),
                audit: serde_json::json!({
                    "kind": "turn_start",
                    "skipped": "empty-session-id",
                    "output": { "emitted": true, "bytes": notice.len(), "text": notice },
                }),
            });
        }
        return Ok(HookContextResult {
            context: None,
            audit: serde_json::json!({
                "kind": "turn_start",
                "skipped": "empty-session-id",
                "output": { "emitted": false, "bytes": 0, "text": null },
            }),
        });
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
                return Ok(HookContextResult {
                    context: Some(notice.clone()),
                    audit: serde_json::json!({
                        "kind": "turn_start",
                        "daemon_rpc_error": format!("{e:#}"),
                        "output": { "emitted": true, "bytes": notice.len(), "text": notice },
                    }),
                });
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
        return Ok(HookContextResult {
            context: Some(ctx),
            audit: v["audit"].clone(),
        });
    }
    Ok(HookContextResult {
        context: None,
        audit: v["audit"].clone(),
    })
}

/// Mid-turn check for PostToolUse hooks. Thin client: the daemon peeks the
/// inbox and computes the rate-limited sibling-session delta. Goes through
/// the sync `daemon::blocking` client, so the actual RPC runs on a blocking
/// thread under [`super::run_hook_blocking`]'s deadline rather than the async
/// hook path's — hooks must never hang regardless of which client an RPC uses.
pub(super) async fn turn_check(
    session: Option<String>,
    emit: EmitFormat,
) -> Result<HookContextResult> {
    if crate::daemon::is_inhibited() {
        return Ok(HookContextResult {
            context: None,
            audit: serde_json::json!({
                "kind": "turn_check",
                "skipped": "daemon-inhibited",
                "output": { "emitted": false, "bytes": 0, "text": null },
            }),
        });
    }
    super::run_hook_blocking(move || {
        let params = crate::cli::rpc_params(serde_json::json!({ "session": session }));
        let v = crate::daemon::blocking::call("turn_check", params)?;
        if let Some(ctx) = v["context"].as_str() {
            emit_context(ctx, emit);
            return Ok(HookContextResult {
                context: Some(ctx.to_string()),
                audit: v["audit"].clone(),
            });
        }
        Ok(HookContextResult {
            context: None,
            audit: v["audit"].clone(),
        })
    })
    .await
}

fn emit_context(content: &str, emit: EmitFormat) {
    println!("{}", render_context_output(content, emit));
}

fn render_context_output(content: &str, emit: EmitFormat) -> String {
    match emit {
        EmitFormat::PlainText => content.to_string(),
        EmitFormat::HookSpecificAdditionalContext { hook_event_name } => {
            let obj = serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": hook_event_name,
                    "additionalContext": content,
                }
            });
            obj.to_string()
        }
    }
}

pub(super) async fn turn_end(session: String) -> Result<()> {
    if session.is_empty() || crate::daemon::is_inhibited() {
        return Ok(());
    }
    super::run_hook_blocking(move || {
        crate::daemon::blocking::call("turn_end", serde_json::json!({"session": session}))?;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_specific_context_uses_additional_context_envelope() {
        let rendered = render_context_output(
            "fabric snapshot",
            EmitFormat::HookSpecificAdditionalContext {
                hook_event_name: "UserPromptSubmit",
            },
        );
        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(
            parsed["hookSpecificOutput"]["hookEventName"],
            "UserPromptSubmit"
        );
        assert_eq!(
            parsed["hookSpecificOutput"]["additionalContext"],
            "fabric snapshot"
        );
        assert!(parsed.get("systemMessage").is_none());
    }

    #[test]
    fn plain_context_stays_plain_text() {
        assert_eq!(
            render_context_output("fabric snapshot", EmitFormat::PlainText),
            "fabric snapshot"
        );
    }
}
