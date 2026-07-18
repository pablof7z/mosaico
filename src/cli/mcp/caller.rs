//! Resolve authenticated remote MCP conversations to first-class fabric sessions.

use anyhow::{Context, Result};
use axum::http::HeaderMap;
use serde_json::{json, Value};

struct Descriptor {
    kind: &'static str,
    source: &'static str,
    fields: Vec<String>,
}

pub(super) async fn resolve(
    auth: &super::auth::AuthState,
    authenticated: &super::auth::Authenticated,
    headers: &HeaderMap,
    params: &Value,
    write: bool,
) -> Result<String> {
    let descriptor = describe(
        authenticated,
        headers,
        params,
        write_requires_conversation(params, write),
    )?;
    let fields = descriptor
        .fields
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let actor_key = auth.redact_actor_key(&fields);
    eprintln!(
        "[mosaico mcp actor] kind={} source={} actor_key={} write={write}",
        descriptor.kind, descriptor.source, actor_key
    );
    let channel = crate::daemon::workspace_path::channel_for_path_or_bail(
        &std::env::current_dir().context("resolving MCP working directory")?,
    )?;
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    let value = client
        .call(
            "mcp_actor_resolve",
            json!({
                "actor_key": actor_key,
                "actor_kind": descriptor.kind,
                "channel": channel,
            }),
        )
        .await?;
    value["pubkey"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .context("daemon returned no MCP actor pubkey")
}

fn write_requires_conversation(params: &Value, write: bool) -> bool {
    let explicit_override = params
        .get("arguments")
        .and_then(|arguments| arguments.get("session"))
        .and_then(Value::as_str)
        .is_some_and(|session| !session.trim().is_empty());
    write && !explicit_override
}

fn describe(
    authenticated: &super::auth::Authenticated,
    headers: &HeaderMap,
    params: &Value,
    write: bool,
) -> Result<Descriptor> {
    let meta = params.get("_meta").and_then(Value::as_object);
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
    };
    let metadata = |name: &str| {
        meta.and_then(|values| values.get(name))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
    };
    let user_agent = header("user-agent").unwrap_or_default();
    let (openai_session, openai_subject, openai_org, source) = if meta.is_some() {
        (
            metadata("openai/session"),
            metadata("openai/subject").unwrap_or(&authenticated.subject),
            metadata("openai/organization").unwrap_or_default(),
            "_meta",
        )
    } else {
        (
            header("x-openai-session"),
            header("x-openai-subject").unwrap_or(&authenticated.subject),
            header("x-openai-organization").unwrap_or_default(),
            "headers",
        )
    };

    let descriptor =
        if openai_session.is_some() || user_agent.to_ascii_lowercase().contains("openai-mcp") {
            if write && openai_session.is_none() {
                anyhow::bail!("OpenAI MCP writes require a stable conversation session identifier");
            }
            Descriptor {
                kind: "openai",
                source,
                fields: [
                    "openai-v1",
                    authenticated.subject.as_str(),
                    openai_org,
                    openai_subject,
                    openai_session.unwrap_or("subject-scoped-read"),
                ]
                .into_iter()
                .map(ToString::to_string)
                .collect(),
            }
        } else if user_agent.to_ascii_lowercase().contains("grok") {
            Descriptor {
                kind: "grok",
                source: "client",
                fields: vec!["grok-global-v1".to_string()],
            }
        } else {
            anyhow::bail!("remote MCP client supplied no supported caller identity metadata");
        };
    Ok(descriptor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authenticated() -> super::super::auth::Authenticated {
        super::super::auth::Authenticated {
            subject: "oauth-human".into(),
        }
    }

    #[test]
    fn openai_headers_select_a_conversation_scoped_actor() {
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "openai-mcp/1.0.0".parse().unwrap());
        headers.insert("x-openai-subject", "opaque-subject".parse().unwrap());
        headers.insert("x-openai-session", "opaque-session".parse().unwrap());
        let descriptor = describe(&authenticated(), &headers, &json!({}), true).unwrap();
        assert_eq!(descriptor.kind, "openai");
        assert_eq!(descriptor.source, "headers");
        assert_eq!(descriptor.fields.last().unwrap(), "opaque-session");
    }

    #[test]
    fn openai_meta_is_canonical_over_transport_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "openai-mcp/1.0.0".parse().unwrap());
        headers.insert("x-openai-session", "header-session".parse().unwrap());
        let params = json!({ "_meta": { "openai/session": "meta-session" } });
        let descriptor = describe(&authenticated(), &headers, &params, true).unwrap();
        assert_eq!(descriptor.source, "_meta");
        assert_eq!(descriptor.fields.last().unwrap(), "meta-session");
    }

    #[test]
    fn grok_actor_is_global_across_oauth_users() {
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "grok-mcp/1.0.0".parse().unwrap());
        let first = describe(&authenticated(), &headers, &json!({}), false).unwrap();
        let second = describe(
            &super::super::auth::Authenticated {
                subject: "another-human".into(),
            },
            &headers,
            &json!({}),
            false,
        )
        .unwrap();
        assert_eq!(first.fields, second.fields);
    }

    #[test]
    fn openai_write_without_conversation_id_fails_closed() {
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "openai-mcp/1.0.0".parse().unwrap());
        let error = describe(&authenticated(), &headers, &json!({}), true)
            .err()
            .unwrap();
        assert!(error.to_string().contains("stable conversation"));
    }

    #[test]
    fn explicit_operator_session_exempts_write_from_conversation_requirement() {
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "openai-mcp/1.0.0".parse().unwrap());
        let params = json!({ "arguments": { "session": "@exact-codex" } });
        assert!(!write_requires_conversation(&params, true));
        assert!(describe(&authenticated(), &headers, &params, false).is_ok());
    }
}
