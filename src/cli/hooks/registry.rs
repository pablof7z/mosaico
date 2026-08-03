use super::observation::find_ancestor_harness;
use crate::cli::turn::EmitFormat;

/// How context blocks are returned to the model by a given harness.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum HookOutputFormat {
    /// Plain text on stdout — Claude Code UserPromptSubmit and most harnesses.
    PlainText,
    /// Codex reads model-visible hook context from event-specific JSON output.
    HookSpecificAdditionalContext,
    /// Hermes plugin hooks consume a compact `{"context":"..."}` object.
    ContextObject,
}

pub(super) struct HostDef {
    pub(super) name: &'static str,
    pub(super) agent_slug: &'static str,
    pub(super) session_id_fields: &'static [&'static str],
    pub(super) session_id_env: Option<&'static str>,
    pub(super) output_format: HookOutputFormat,
    pub(super) requires_harness_session: bool,
}

static HOOK_HOSTS: &[HostDef] = &[
    HostDef {
        name: "claude-code",
        agent_slug: "claude",
        session_id_fields: &["session_id"],
        session_id_env: None,
        output_format: HookOutputFormat::PlainText,
        requires_harness_session: true,
    },
    HostDef {
        name: "codex",
        agent_slug: "codex",
        session_id_fields: &["session_id"],
        session_id_env: None,
        output_format: HookOutputFormat::HookSpecificAdditionalContext,
        requires_harness_session: true,
    },
    HostDef {
        name: "opencode",
        agent_slug: "opencode",
        session_id_fields: &["session_id"],
        session_id_env: None,
        output_format: HookOutputFormat::PlainText,
        requires_harness_session: false,
    },
    HostDef {
        name: "grok",
        agent_slug: "grok",
        session_id_fields: &["session_id"],
        session_id_env: Some("GROK_SESSION_ID"),
        output_format: HookOutputFormat::PlainText,
        requires_harness_session: true,
    },
    HostDef {
        name: "goose",
        agent_slug: "goose",
        session_id_fields: &["session_id"],
        session_id_env: None,
        // Goose ignores hook stdout. Mosaico also publishes the same context
        // into the session-specific Top Of Mind file after this call.
        output_format: HookOutputFormat::PlainText,
        requires_harness_session: true,
    },
    HostDef {
        name: "hermes",
        agent_slug: "hermes",
        session_id_fields: &["session_id"],
        session_id_env: None,
        output_format: HookOutputFormat::ContextObject,
        requires_harness_session: true,
    },
    HostDef {
        name: "kimi",
        agent_slug: "kimi",
        session_id_fields: &["session_id"],
        session_id_env: None,
        output_format: HookOutputFormat::PlainText,
        requires_harness_session: true,
    },
];

pub(super) fn find_hook_host(name: &str) -> Option<&'static HostDef> {
    if name == "help" {
        eprintln!(
            "known hosts: {}",
            HOOK_HOSTS
                .iter()
                .map(|host| host.name)
                .collect::<Vec<_>>()
                .join(", ")
        );
        return None;
    }
    HOOK_HOSTS.iter().find(|host| host.name == name)
}

pub(super) fn emit_format(host: &HostDef, hook_type: &str) -> EmitFormat {
    if host.name == "kimi" && hook_type == "stop" {
        return EmitFormat::KimiStopBlock;
    }
    if host.name == "claude-code" && hook_type == "post-tool-use" {
        return EmitFormat::HookSpecificAdditionalContext {
            hook_event_name: "PostToolUse",
        };
    }
    match host.output_format {
        HookOutputFormat::PlainText => EmitFormat::PlainText,
        HookOutputFormat::HookSpecificAdditionalContext => {
            EmitFormat::HookSpecificAdditionalContext {
                hook_event_name: hook_event_name(hook_type),
            }
        }
        HookOutputFormat::ContextObject => EmitFormat::ContextObject,
    }
}

fn hook_event_name(hook_type: &str) -> &'static str {
    match hook_type {
        "session-start" => "SessionStart",
        "session-end" => "SessionEnd",
        "user-prompt-submit" => "UserPromptSubmit",
        "post-tool-use" => "PostToolUse",
        "pre-tool-use" => "PreToolUse",
        "stop" => "Stop",
        _ => "Unknown",
    }
}

pub(super) fn caller_watch_pid_anchor() -> Option<(&'static str, i32)> {
    find_ancestor_harness()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::turn::render_context_output;

    #[test]
    fn harness_context_envelopes_render_current_adapter_contracts() {
        for host in ["claude-code", "grok", "opencode", "goose", "kimi"] {
            let format = emit_format(find_hook_host(host).unwrap(), "user-prompt-submit");
            assert_eq!(render_context_output("fabric", format), "fabric");
        }

        let claude = render_context_output(
            "fabric",
            emit_format(find_hook_host("claude-code").unwrap(), "post-tool-use"),
        );
        let claude: serde_json::Value = serde_json::from_str(&claude).unwrap();
        assert_eq!(claude["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        assert_eq!(claude["hookSpecificOutput"]["additionalContext"], "fabric");

        let codex = render_context_output(
            "fabric",
            emit_format(find_hook_host("codex").unwrap(), "user-prompt-submit"),
        );
        let codex: serde_json::Value = serde_json::from_str(&codex).unwrap();
        assert_eq!(
            codex["hookSpecificOutput"]["hookEventName"],
            "UserPromptSubmit"
        );
        assert_eq!(codex["hookSpecificOutput"]["additionalContext"], "fabric");

        let hermes = render_context_output(
            "fabric",
            emit_format(find_hook_host("hermes").unwrap(), "user-prompt-submit"),
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&hermes).unwrap(),
            serde_json::json!({"context": "fabric"})
        );

        let kimi = render_context_output(
            "fabric",
            emit_format(find_hook_host("kimi").unwrap(), "stop"),
        );
        let kimi: serde_json::Value = serde_json::from_str(&kimi).unwrap();
        assert_eq!(
            kimi["hookSpecificOutput"]["permissionDecisionReason"],
            "fabric"
        );
    }
}
