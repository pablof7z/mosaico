use super::*;

pub(super) async fn channel_create(
    path: String,
    about: String,
    agents: Vec<String>,
    session: Option<String>,
) -> Result<()> {
    let target = create_target(&path)?;
    let parsed = parse_agents(&agents)?;
    let v = daemon_call_async(
        "channel_create",
        crate::cli::rpc_params(with_session(
            serde_json::json!({
                "parent_channel": target.parent_channel,
                "name": target.name,
                "about": about,
                "agents": parsed,
            }),
            session.as_deref(),
        )),
    )
    .await?;
    if let Some(refs) = v["ambiguous"].as_array() {
        print_ambiguous_create(&path, &about, &agents, session.as_deref(), refs, &v);
    }

    let oid = v["orchestration_event_id"].as_str().unwrap_or("");
    let switched = v["switched"].as_bool().unwrap_or(false);
    if switched {
        println!("#{path} created and switched to it");
    } else {
        println!("#{path} created");
    }
    if !oid.is_empty() {
        println!("  orchestration kind:9 {}", &oid[..oid.len().min(8)]);
    }
    Ok(())
}

struct CreateTarget {
    parent_channel: Option<String>,
    name: String,
}

fn create_target(path: &str) -> Result<CreateTarget> {
    let path = path.trim();
    if !path.starts_with('/') || path.contains('.') || path.ends_with('/') || path.contains("//") {
        anyhow::bail!("channel create <path> requires a full absolute path, e.g. /workspace/child");
    }
    let segments: Vec<&str> = path[1..].split('/').map(str::trim).collect();
    if segments.iter().any(|segment| segment.is_empty()) {
        anyhow::bail!("channel create <path> requires a full absolute path, e.g. /workspace/child");
    }
    let Some(name) = segments.last() else {
        anyhow::bail!("channel create <path> requires a non-empty path");
    };
    let Some(parent_segments) = segments.get(..segments.len() - 1).filter(|s| !s.is_empty()) else {
        anyhow::bail!(
            "channel create <path> needs a parent, e.g. /workspace/{name}; the workspace itself \
             comes from `channel init`, not `channel create`"
        );
    };
    Ok(CreateTarget {
        parent_channel: Some(format!("/{}", parent_segments.join("/"))),
        name: (*name).to_string(),
    })
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

fn print_ambiguous_create(
    path: &str,
    about: &str,
    agents: &[String],
    session: Option<&str>,
    refs: &[serde_json::Value],
    response: &serde_json::Value,
) -> ! {
    let reference = response["reference"].as_str().unwrap_or(path);
    let leaf = create_target(path)
        .ok()
        .map(|target| target.name)
        .unwrap_or_else(|| path.to_string());
    eprintln!("'{reference}' is ambiguous — re-run with an exact path:");
    for r in refs.iter().filter_map(|r| r.as_str()) {
        let full_path = format!("{r}/{leaf}");
        let mut cmd = format!(
            "  mosaico channel create {} --about {}",
            shell_quote(&full_path),
            shell_quote(about)
        );
        for agent in agents {
            cmd.push_str(" --agent ");
            cmd.push_str(&shell_quote(agent));
        }
        if let Some(session) = session {
            cmd.push_str(" --session ");
            cmd.push_str(&shell_quote(session));
        }
        eprintln!("{cmd}");
    }
    std::process::exit(2);
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_target_splits_leaf_from_absolute_parent_path() {
        let target = create_target("/workspace/epic/planning").unwrap();
        assert_eq!(target.parent_channel.as_deref(), Some("/workspace/epic"));
        assert_eq!(target.name, "planning");
    }

    #[test]
    fn create_target_accepts_deeper_absolute_paths() {
        let target = create_target("/workspace/epic/planning/research").unwrap();
        assert_eq!(
            target.parent_channel.as_deref(),
            Some("/workspace/epic/planning")
        );
        assert_eq!(target.name, "research");
    }

    #[test]
    fn create_target_rejects_relative_paths() {
        let error = match create_target("epic/planning") {
            Ok(_) => panic!("a relative path must be rejected"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("full absolute path"));
    }

    #[test]
    fn create_target_rejects_a_bare_workspace_with_no_parent() {
        let error = match create_target("/workspace") {
            Ok(_) => panic!("a bare workspace path has no parent to create under"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("channel init"));
    }

    #[test]
    fn create_target_rejects_dotted_paths() {
        let error = match create_target("/workspace/epic.planning") {
            Ok(_) => panic!("dotted path must be rejected"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("full absolute path"));
    }
}
