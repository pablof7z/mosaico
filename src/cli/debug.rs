mod data;
mod loader;
mod render;
mod state;
mod util;

// Re-export public items for cli.rs to use
pub use data::HookTailOpts;

// Wrapper to match the original signature
pub(super) fn hook_tail(opts: HookTailOpts) -> anyhow::Result<()> {
    state::hook_tail(opts)
}

pub(super) async fn outbox(live: bool, limit: u64, refresh: std::time::Duration) -> anyhow::Result<()> {
    loop {
        let v = super::daemon_call_async("debug_outbox", serde_json::json!({ "limit": limit }))
            .await?;
        render_outbox(&v);
        if !live {
            break;
        }
        tokio::time::sleep(refresh).await;
    }
    Ok(())
}

fn render_outbox(v: &serde_json::Value) {
    let rows = v["rows"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    if rows.is_empty() {
        println!("status outbox: empty");
        return;
    }
    println!(
        "{:<13} {:<8} {:<10} {:<7} {:<18} {:<20} title",
        "state", "version", "retries", "busy", "session", "project"
    );
    for r in rows {
        let session = r["session_id"].as_str().unwrap_or("");
        let short = crate::util::SessionId::from(session).to_string();
        let title = r["title"].as_str().unwrap_or("");
        println!(
            "{:<13} {:<8} {:<10} {:<7} {:<18} {:<20} {}",
            r["publish_state"].as_str().unwrap_or(""),
            r["state_version"].as_i64().unwrap_or_default(),
            r["retries"].as_i64().unwrap_or_default(),
            r["busy"].as_bool().unwrap_or(false),
            short,
            r["project"].as_str().unwrap_or(""),
            title,
        );
        if let Some(err) = r["last_error"].as_str().filter(|s| !s.is_empty()) {
            println!("  error: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::data::SessionPane;
    use super::util::*;
    use std::collections::BTreeMap;

    #[test]
    fn command_session_prefers_env_then_explicit_flag() {
        let v = serde_json::json!({
            "env": {"TENEX_EDGE_SESSION": "env-session"},
            "command": {"explicit_session": "flag-session"},
            "process": {"cwd": "/tmp"}
        });
        assert_eq!(command_session(&v).as_deref(), Some("env-session"));

        let v = serde_json::json!({
            "env": {},
            "command": {"explicit_session": "flag-session"},
            "process": {"cwd": "/tmp"}
        });
        assert_eq!(command_session(&v).as_deref(), Some("flag-session"));
    }

    #[test]
    fn hook_session_accepts_codex_field_variants() {
        let v = serde_json::json!({"conversation_id": "codex-session"});
        assert_eq!(hook_session(&v).as_deref(), Some("codex-session"));
    }

    #[test]
    fn command_session_can_infer_unique_live_agent_project() {
        let mut panes = BTreeMap::new();
        panes.insert(
            "session-a".to_string(),
            SessionPane {
                session: "session-a".to_string(),
                project: "proj".to_string(),
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
                project: "proj".to_string(),
                agent: "coder".to_string(),
                ..SessionPane::default()
            },
        );
        assert_eq!(infer_command_session(&panes, "coder", "proj"), None);
    }
}
