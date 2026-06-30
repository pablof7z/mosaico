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
    fn resolve_env_session(verb: &str) -> Result<String> {
        std::env::var("TENEX_EDGE_SESSION")
            .ok()
            .filter(|s| !s.is_empty())
            .with_context(|| format!("{verb} must be run from within a tenex-edge agent session (TENEX_EDGE_SESSION is not set)"))
    }
    fn print_ambiguous(verb: &str, channel: &str, v: &serde_json::Value) -> ! {
        let name = v["reference"].as_str().unwrap_or(channel);
        eprintln!("'{name}' is ambiguous — re-run with an exact path:");
        if let Some(refs) = v["ambiguous"].as_array() {
            for r in refs.iter().filter_map(|r| r.as_str()) {
                eprintln!("  tenex-edge channels {verb} {r}");
            }
        }
        std::process::exit(2);
    }
    match action {
        ChannelsAction::Create {
            name,
            about,
            agents,
            parent_channel,
            message,
        } => {
            // `--agent` is optional: an agent may carve out an empty channel and
            // populate it later. Each `slug@backend` splits on the LAST `@` (agent
            // slugs never contain `@`).
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
                    // No `parent` here: the daemon defaults the new channel under
                    // the creating session's CURRENT channel. `--parent-channel`
                    // overrides that with a project-relative reference.
                    "parent_channel": parent_channel,
                    "name": name,
                    "about": about.unwrap_or_default(),
                    "agents": parsed,
                    "brief": brief,
                    // Caller identity so the daemon auto-adds the creating agent
                    // to the new room AND auto-switches its session into it
                    // (resolved like the messaging commands).
                    "agent": crate::cli::agent_env_slug(),
                    "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
                    "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
                }),
            )
            .await?;
            // Ambiguous `--parent-channel`: the daemon returns candidate paths
            // instead of creating. Print copy-paste re-runs and exit 2.
            if let Some(refs) = v["ambiguous"].as_array() {
                let name = v["reference"].as_str().unwrap_or("");
                eprintln!("'{name}' is ambiguous — re-run with an exact --parent-channel:");
                for r in refs.iter().filter_map(|r| r.as_str()) {
                    eprintln!("  tenex-edge channels create --name {name} --parent-channel {r}");
                }
                std::process::exit(2);
            }
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
            if v["switched"].as_bool().unwrap_or(false) {
                println!("  switched to it");
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
                // Name-first: the human handle is primary; the opaque id is shown
                // dimmed only as a secondary locator, and alone only when the
                // channel has no name yet.
                if name.is_empty() {
                    println!("{indent}{}", id.if_supports_color(Stdout, |s| s.cyan()));
                } else {
                    let name_c = name.if_supports_color(Stdout, |s| s.bold());
                    let id_c = id.if_supports_color(Stdout, |s| s.cyan());
                    println!("{indent}{name_c}  ({id_c})");
                }
            }
        }
        ChannelsAction::Join { channel } => {
            let env_session = resolve_env_session("channels join")?;
            let v = daemon_call_async(
                "channels_join",
                serde_json::json!({
                    "channel": channel.clone(),
                    "env_session": env_session,
                }),
            )
            .await?;
            if v["ambiguous"].is_array() {
                print_ambiguous("join", &channel, &v);
            }
            println!("joined channel {}", v["channel"].as_str().unwrap_or(&channel));
        }
        ChannelsAction::Leave { channel } => {
            let env_session = resolve_env_session("channels leave")?;
            let v = daemon_call_async(
                "channels_leave",
                serde_json::json!({
                    "channel": channel.clone(),
                    "env_session": env_session,
                }),
            )
            .await?;
            if v["ambiguous"].is_array() {
                print_ambiguous("leave", &channel, &v);
            }
            println!("left channel {}", v["channel"].as_str().unwrap_or(&channel));
        }
        ChannelsAction::Switch { channel } => {
            let env_session = resolve_env_session("channels switch")?;
            let v = daemon_call_async(
                "channels_switch",
                serde_json::json!({
                    "channel": channel.clone(),
                    "env_session": env_session,
                }),
            )
            .await?;
            // Ambiguous reference: the daemon returns the candidate paths instead
            // of switching. Print them as copy-paste-ready re-runs and exit 2 so a
            // calling agent can branch on the code without parsing prose.
            if v["ambiguous"].is_array() {
                print_ambiguous("switch", &channel, &v);
            }
            println!("switched to channel {}", channel);
        }
    }
    Ok(())
}

// ── invite (spawn a fresh session into the current channel) ──────────────────

/// `tenex-edge invite <slug[@backend]>` — spawn a fresh session for an agent into
/// the channel this command runs in. Forwards the caller's session-env signals so
/// the daemon can pin the spawn to the inviter's current `channel_h`.
pub async fn invite(agent: String) -> Result<()> {
    let v = daemon_call_async(
        "invite",
        serde_json::json!({
            "agent": agent,
            "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
            "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
            "agent_slug": crate::cli::agent_env_slug(),
        }),
    )
    .await?;
    let slug = v["agent"].as_str().unwrap_or(&agent);
    let pane = v["pane_id"].as_str().unwrap_or("?");
    // Never print the opaque channel_h (same no-leak rule as the fabric format).
    println!(
        "invited {} into your current channel — fresh session spawned (pane {})",
        slug.bold(),
        pane.dimmed()
    );
    Ok(())
}
