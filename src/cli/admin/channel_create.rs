use super::*;

pub(super) async fn channel_create(
    path: String,
    about: String,
    agents: Vec<String>,
    session: Option<String>,
) -> Result<()> {
    crate::channel_ref::split_create_path(&path)?;
    let parsed = parse_agents(&agents)?;
    let v = daemon_call_async(
        "channel_create",
        crate::cli::rpc_params(with_session(
            serde_json::json!({
                "channel": path,
                "about": about,
                "agents": parsed,
            }),
            session.as_deref(),
        )),
    )
    .await?;
    let oid = v["orchestration_event_id"].as_str().unwrap_or("");
    if v["joined"].as_bool().unwrap_or(false) {
        println!("{path} created and joined");
    } else {
        println!("{path} created");
    }
    if !oid.is_empty() {
        println!("  orchestration kind:9 {}", &oid[..oid.len().min(8)]);
    }
    Ok(())
}

fn parse_agents(agents: &[String]) -> Result<Vec<serde_json::Value>> {
    let mut parsed: Vec<serde_json::Value> = Vec::with_capacity(agents.len());
    for a in agents {
        let target = crate::idref::parse_agent_backend_ref(a)
            .with_context(|| format!("malformed --agent {a:?}: expected slug@backend-label"))?;
        let backend = target
            .backend
            .with_context(|| format!("malformed --agent {a:?}: expected slug@backend-label"))?;
        parsed.push(serde_json::json!({ "slug": target.slug, "backend": backend }));
    }
    Ok(parsed)
}

fn with_session(mut params: serde_json::Value, session: Option<&str>) -> serde_json::Value {
    if let Some(session) = session.filter(|s| !s.is_empty()) {
        if let Some(obj) = params.as_object_mut() {
            obj.insert("session".into(), serde_json::json!(session));
        }
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_agents_requires_backend_qualified_targets() {
        assert!(parse_agents(&["agent".into()]).is_err());
        assert_eq!(
            parse_agents(&["agent@laptop".into()]).unwrap(),
            vec![serde_json::json!({"slug": "agent", "backend": "laptop"})]
        );
    }
}
