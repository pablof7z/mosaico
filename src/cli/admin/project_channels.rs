use super::*;

// ── project ──────────────────────────────────────────────────────────────────

pub async fn project(action: ProjectAction) -> Result<()> {
    match action {
        ProjectAction::List => {
            let v = daemon_call_async("project_list", serde_json::json!({})).await?;
            let projects = v["projects"]
                .as_array()
                .map(|a| a.as_slice())
                .unwrap_or(&[]);
            if projects.is_empty() {
                println!("No NIP-29 groups found on the relay.");
                return Ok(());
            }
            let max_slug = projects
                .iter()
                .filter_map(|p| p["slug"].as_str())
                .map(|s| s.len())
                .max()
                .unwrap_or(0);
            for p in projects {
                let slug = p["slug"].as_str().unwrap_or("");
                let about = p["about"].as_str().unwrap_or("");
                if about.is_empty() {
                    println!("{slug}");
                } else {
                    println!("{slug:<max_slug$}  — {about}");
                }
            }
        }
        ProjectAction::Init { force } => {
            let cwd = std::env::current_dir().unwrap_or_default();
            let (slug, path) = crate::project::register_project(&cwd, force)?;
            println!("initialized project {slug} at {}", path.display());
        }
        ProjectAction::Edit {
            description,
            project,
        } => {
            let slug = match project {
                Some(p) => p,
                None => {
                    crate::project::resolve_or_bail(&std::env::current_dir().unwrap_or_default())?
                }
            };
            let v = daemon_call_async(
                "project_edit",
                serde_json::json!({ "project": slug, "description": description }),
            )
            .await?;
            let event_id = v["event_id"].as_str().unwrap_or("?");
            println!("Updated {slug}: {}", &event_id[..event_id.len().min(8)]);
        }
        ProjectAction::Add { project, pubkey } => match pubkey {
            Some(pubkey) => {
                let project = match project {
                    Some(p) => p,
                    None => crate::project::resolve_or_bail(
                        &std::env::current_dir().unwrap_or_default(),
                    )?,
                };
                let v = daemon_call_async(
                    "project_add",
                    serde_json::json!({ "project": project, "pubkey": pubkey }),
                )
                .await?;
                let resolved = v["pubkey"].as_str().unwrap_or(&pubkey);
                println!(
                    "added {} to {}",
                    pubkey_short(resolved).cyan(),
                    project.bold()
                );
            }
            None => {
                let project = match project {
                    Some(p) => p,
                    None => crate::project::resolve_or_bail(
                        &std::env::current_dir().unwrap_or_default(),
                    )?,
                };
                super::super::project_agents::edit_membership(project).await?;
            }
        },
    }
    Ok(())
}

// ── channels (NIP-29 subgroup task rooms) ────────────────────────────────────

pub async fn channels(action: ChannelsAction) -> Result<()> {
    fn resolve_project(project: Option<String>) -> Result<String> {
        match project {
            Some(p) => Ok(p),
            None => crate::project::resolve_or_bail(&std::env::current_dir().unwrap_or_default()),
        }
    }
    match action {
        ChannelsAction::Create {
            name,
            agents,
            project,
            message,
        } => {
            let parent = resolve_project(project)?;
            if agents.is_empty() {
                bail!("at least one --agent slug@backend is required");
            }
            // Parse each `slug@backend` on the LAST `@` (agent slugs never contain `@`).
            let mut parsed: Vec<serde_json::Value> = Vec::with_capacity(agents.len());
            for a in &agents {
                let (slug, backend) = a
                    .rsplit_once('@')
                    .filter(|(s, b)| !s.is_empty() && !b.is_empty())
                    .with_context(|| format!("malformed --agent {a:?}: expected slug@backend"))?;
                parsed.push(serde_json::json!({ "slug": slug, "backend": backend }));
            }
            let brief = match &message {
                Some(path) => std::fs::read_to_string(path)
                    .with_context(|| format!("reading --message {}", path.display()))?,
                None => String::new(),
            };
            let v = daemon_call_async(
                "channels_create",
                serde_json::json!({
                    "parent": parent,
                    "name": name,
                    "agents": parsed,
                    "brief": brief,
                    // Caller identity so the daemon auto-adds the creating agent
                    // to the new room (resolved like the messaging commands).
                    "agent": crate::cli::agent_env_slug(),
                    "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
                    "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
                }),
            )
            .await?;
            let child = v["child_h"].as_str().unwrap_or("?");
            let path = v["display_path"].as_str().unwrap_or("");
            let oid = v["orchestration_event_id"].as_str().unwrap_or("");
            println!("created channel {} ({})", child.bold(), path);
            if let Some(admins) = v["admins"].as_array() {
                println!("  admins copied: {}", admins.len());
            }
            if let Some(joined) = v["creator"].as_str() {
                if !joined.is_empty() {
                    println!("  joined as {}", pubkey_short(joined).cyan());
                }
            }
            if !oid.is_empty() {
                println!("  orchestration kind:9 {}", &oid[..oid.len().min(8)]);
            }
        }
        ChannelsAction::List { project } => {
            use owo_colors::Stream::Stdout;
            let parent = resolve_project(project)?;
            let v = daemon_call_async("channels_list", serde_json::json!({ "project": parent }))
                .await?;
            let rooms = v["rooms"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
            // Root of the tree is the project itself. Colorize ONLY when stdout is a
            // real terminal: piped/captured output (the e2e harness, shell
            // substitution) must be plain so callers can match the slug literally —
            // `.bold()` would otherwise wrap it in ANSI escapes and a `^slug$` grep
            // would never match.
            println!("{}", parent.if_supports_color(Stdout, |s| s.bold()));
            if rooms.is_empty() {
                println!("  (no channels)");
                return Ok(());
            }
            for r in rooms {
                let id = r["child_h"].as_str().unwrap_or("");
                let name = r["name"].as_str().unwrap_or("");
                let depth = r["depth"].as_u64().unwrap_or(0) as usize;
                // depth 0 = direct child of the project root → one level of indent.
                let indent = "  ".repeat(depth + 1);
                let id_c = id.if_supports_color(Stdout, |s| s.cyan());
                if name.is_empty() {
                    println!("{indent}{id_c}");
                } else {
                    println!("{indent}{id_c}  — {name}");
                }
            }
        }
        ChannelsAction::Switch { channel } => {
            let env_session = std::env::var("TENEX_EDGE_SESSION")
                .ok()
                .filter(|s| !s.is_empty())
                .context("channels switch must be run from within a tenex-edge agent session (TENEX_EDGE_SESSION is not set)")?;
            daemon_call_async(
                "channels_switch",
                serde_json::json!({
                    "channel": channel,
                    "env_session": env_session,
                }),
            )
            .await?;
            println!("switched to channel {}", channel);
        }
    }
    Ok(())
}
