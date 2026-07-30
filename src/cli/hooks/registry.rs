use super::observation::find_ancestor_pid;
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
    pub(super) pid_search: Option<&'static str>,
    pub(super) requires_harness_session: bool,
}

static HOOK_HOSTS: &[HostDef] = &[
    HostDef {
        name: "claude-code",
        agent_slug: "claude",
        session_id_fields: &["session_id"],
        session_id_env: None,
        output_format: HookOutputFormat::PlainText,
        pid_search: Some("claude"),
        requires_harness_session: true,
    },
    HostDef {
        name: "codex",
        agent_slug: "codex",
        session_id_fields: &["session_id"],
        session_id_env: None,
        output_format: HookOutputFormat::HookSpecificAdditionalContext,
        pid_search: Some("codex"),
        requires_harness_session: true,
    },
    HostDef {
        name: "opencode",
        agent_slug: "opencode",
        session_id_fields: &["session_id"],
        session_id_env: None,
        output_format: HookOutputFormat::PlainText,
        pid_search: None,
        requires_harness_session: false,
    },
    HostDef {
        name: "grok",
        agent_slug: "grok",
        session_id_fields: &["session_id"],
        session_id_env: Some("GROK_SESSION_ID"),
        output_format: HookOutputFormat::PlainText,
        pid_search: Some("grok"),
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
        pid_search: Some("goose"),
        requires_harness_session: true,
    },
    HostDef {
        name: "hermes",
        agent_slug: "hermes",
        session_id_fields: &["session_id"],
        session_id_env: None,
        output_format: HookOutputFormat::ContextObject,
        pid_search: Some("hermes"),
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
    HOOK_HOSTS
        .iter()
        .filter_map(|host| host.pid_search.map(|needle| (host.name, needle)))
        .find_map(|(name, needle)| find_ancestor_pid(needle).map(|pid| (name, pid)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::turn::render_context_output;

    #[test]
    fn harness_context_envelopes_render_current_adapter_contracts() {
        for host in ["claude-code", "grok", "opencode", "goose"] {
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
    }
}
