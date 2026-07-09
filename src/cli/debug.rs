mod args;
mod data;
mod loader;
mod render;
mod state;
mod util;

// Re-export public items for cli.rs to use
pub(super) use args::{debug, DebugAction};
pub use data::HookTailOpts;

// Wrapper to match the original signature
pub(super) fn hook_tail(opts: HookTailOpts) -> anyhow::Result<()> {
    state::hook_tail(opts)
}

pub(super) async fn outbox(
    live: bool,
    limit: u64,
    refresh: std::time::Duration,
) -> anyhow::Result<()> {
    loop {
        let v =
            super::daemon_call_async("debug_outbox", serde_json::json!({ "limit": limit })).await?;
        render_outbox(&v);
        if !live {
            break;
        }
        tokio::time::sleep(refresh).await;
    }
    Ok(())
}

fn render_outbox(v: &serde_json::Value) {
    print!("{}", render_outbox_text(v));
}

fn render_outbox_text(v: &serde_json::Value) -> String {
    let rows = v["rows"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    if rows.is_empty() {
        return "outbox: empty\n".to_string();
    }
    let mut out = format!(
        "{:<8} {:<9} {:<7} {:<12} {:<6} {:<14} {:<12} content\n",
        "local_id", "state", "retries", "event", "kind", "channel", "author"
    );
    for r in rows {
        let event = r["event_json"]
            .as_str()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
        let event_id = event
            .as_ref()
            .and_then(|v| v["id"].as_str())
            .map(|s| short(s, 12))
            .unwrap_or_else(|| "(invalid)".to_string());
        let kind = event
            .as_ref()
            .and_then(|v| v["kind"].as_i64())
            .map(|kind| kind.to_string())
            .unwrap_or_default();
        let author = event
            .as_ref()
            .and_then(|v| v["pubkey"].as_str())
            .map(|s| short(s, 12))
            .unwrap_or_default();
        let channel = event
            .as_ref()
            .and_then(channel_tag)
            .map(|s| short(&s, 14))
            .unwrap_or_default();
        let content = event
            .as_ref()
            .and_then(|v| v["content"].as_str())
            .map(|s| short(s, 72))
            .unwrap_or_default();
        out.push_str(&format!(
            "{:<8} {:<9} {:<7} {:<12} {:<6} {:<14} {:<12} {}\n",
            r["local_id"].as_i64().unwrap_or_default(),
            r["state"].as_str().unwrap_or(""),
            r["retries"].as_i64().unwrap_or_default(),
            event_id,
            kind,
            channel,
            author,
            content
        ));
        if let Some(err) = r["last_error"].as_str().filter(|s| !s.is_empty()) {
            out.push_str(&format!("  error: {err}\n"));
        }
    }
    out
}

fn channel_tag(event: &serde_json::Value) -> Option<String> {
    event["tags"]
        .as_array()?
        .iter()
        .filter_map(|tag| tag.as_array())
        .find_map(|tag| {
            let key = tag.first().and_then(|v| v.as_str())?;
            let value = tag.get(1).and_then(|v| v.as_str())?;
            (key == "h").then(|| value.to_string())
        })
}

fn short(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let clipped = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{clipped}...")
    } else {
        clipped
    }
}

#[cfg(test)]
mod outbox_render_tests {
    use super::*;

    #[test]
    fn debug_outbox_renders_generic_signed_event_rows() {
        let text = render_outbox_text(&serde_json::json!({
            "rows": [{
                "local_id": 7,
                "state": "pending",
                "retries": 3,
                "last_error": "relay rejected",
                "event_json": serde_json::json!({
                    "id": "abcdef1234567890",
                    "kind": 9,
                    "pubkey": "0123456789abcdef",
                    "content": "hello from the fabric",
                    "tags": [["h", "tenex-edge"]]
                }).to_string(),
                "enqueued_at": 100
            }]
        }));
        assert!(text.contains("local_id"));
        assert!(text.contains("abcdef123456..."));
        assert!(text.contains("tenex-edge"));
        assert!(text.contains("relay rejected"));
        assert!(!text.contains("status outbox"));
    }

    #[test]
    fn debug_outbox_empty_message_is_generic() {
        assert_eq!(
            render_outbox_text(&serde_json::json!({"rows": []})),
            "outbox: empty\n"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::data::SessionPane;
    use super::util::*;
    use std::collections::BTreeMap;

    #[test]
    fn command_session_uses_explicit_flag_only() {
        let v = serde_json::json!({
            "env": {"TENEX_EDGE_SESSION": "env-session"},
            "command": {"explicit_session": "flag-session"},
            "process": {"cwd": "/tmp"}
        });
        assert_eq!(command_session(&v).as_deref(), Some("flag-session"));

        let v = serde_json::json!({
            "env": {},
            "command": {"explicit_session": "flag-session"},
            "process": {"cwd": "/tmp"}
        });
        assert_eq!(command_session(&v).as_deref(), Some("flag-session"));

        let v = serde_json::json!({
            "env": {"TENEX_EDGE_SESSION": "env-session"},
            "command": {},
            "process": {"cwd": "/tmp"}
        });
        assert_eq!(command_session(&v), None);
    }

    #[test]
    fn hook_session_accepts_codex_field_variants() {
        let v = serde_json::json!({"conversation_id": "codex-session"});
        assert_eq!(hook_session(&v).as_deref(), Some("codex-session"));
    }

    #[test]
    fn command_session_can_infer_unique_live_agent_channel() {
        let mut panes = BTreeMap::new();
        panes.insert(
            "session-a".to_string(),
            SessionPane {
                session: "session-a".to_string(),
                root: "proj".to_string(),
                agent: "coder".to_string(),
                ..SessionPane::default()
            },
        );
        assert_eq!(
            infer_command_session(&panes, "coder", "proj").as_deref(),
            Some("session-a")
        );

        panes.insert(
            "session-b".to_string(),
            SessionPane {
                session: "session-b".to_string(),
                root: "proj".to_string(),
                agent: "coder".to_string(),
                ..SessionPane::default()
            },
        );
        assert_eq!(infer_command_session(&panes, "coder", "proj"), None);
    }
}
