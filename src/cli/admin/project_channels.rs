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
        } => {
            // `--agent` is optional: an agent may carve out an empty channel and
            // populate it later. Each target is `slug@backend-label`; the backend
            // side is a tenex-edge config label, not a machine hostname.
            let mut parsed: Vec<serde_json::Value> = Vec::with_capacity(agents.len());
            for a in &agents {
                let target = crate::idref::parse_agent_backend_ref(a).with_context(|| {
                    format!("malformed --agent {a:?}: expected slug@backend-label")
                })?;
                let backend = target.backend.with_context(|| {
                    format!("malformed --agent {a:?}: expected slug@backend-label")
                })?;
                parsed.push(serde_json::json!({ "slug": target.slug, "backend": backend }));
            }
            let v = daemon_call_async(
                "channels_create",
                crate::cli::rpc_params(serde_json::json!({
                    // No `parent` here: the daemon defaults the new channel under
                    // the creating session's CURRENT channel. `--parent-channel`
                    // overrides that with a project-relative reference.
                    "parent_channel": parent_channel,
                    "name": name,
                    "about": about,
                    "agents": parsed,
                })),
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
            let oid = v["orchestration_event_id"].as_str().unwrap_or("");
            let switched = v["switched"].as_bool().unwrap_or(false);
            if switched {
                println!("#{} created and switched to it", name);
            } else {
                println!("#{} created", name);
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
            let v = daemon_call_async(
                "channels_join",
                crate::cli::rpc_params(serde_json::json!({ "channel": channel.clone() })),
            )
            .await?;
            if v["ambiguous"].is_array() {
                print_ambiguous("join", &channel, &v);
            }
            println!(
                "joined channel {}",
                v["channel"].as_str().unwrap_or(&channel)
            );
        }
        ChannelsAction::Leave { channel } => {
            let v = daemon_call_async(
                "channels_leave",
                crate::cli::rpc_params(serde_json::json!({ "channel": channel.clone() })),
            )
            .await?;
            if v["ambiguous"].is_array() {
                print_ambiguous("leave", &channel, &v);
            }
            println!("left channel {}", v["channel"].as_str().unwrap_or(&channel));
        }
        ChannelsAction::Switch { channel } => {
            let v = daemon_call_async(
                "channels_switch",
                crate::cli::rpc_params(serde_json::json!({ "channel": channel.clone() })),
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

// ── invite (spawn/resume into an explicit channel) ───────────────────────────

/// `tenex-edge invite --channel <channel> (--agent <slug[@backend]> | --session <id>)`
/// spawns a fresh agent session or resumes a prior one into an existing channel.
pub async fn invite(channel: String, agent: Option<String>, session: Option<String>) -> Result<()> {
    let selector = agent
        .as_ref()
        .map(|a| format!("--agent {a}"))
        .or_else(|| session.as_ref().map(|s| format!("--session {s}")))
        .unwrap_or_default();
    let v = daemon_call_async(
        "invite",
        crate::cli::rpc_params(serde_json::json!({
            "channel": channel,
            "target_agent": agent,
            "session": session,
        })),
    )
    .await?;
    if v["ambiguous"].is_array() {
        let name = v["reference"].as_str().unwrap_or("");
        eprintln!("'{name}' is ambiguous — re-run with an exact --channel:");
        if let Some(refs) = v["ambiguous"].as_array() {
            for r in refs.iter().filter_map(|r| r.as_str()) {
                eprintln!("  tenex-edge invite --channel {r} {selector}");
            }
        }
        std::process::exit(2);
    }
    let slug = v["agent"].as_str().unwrap_or("session");
    let pane = v["pane_id"].as_str().unwrap_or("");
    let online = v["online_agent"].as_str().unwrap_or(slug);
    if pane.is_empty() {
        println!("{} is now online", online.bold());
    } else {
        println!("{} is now online (pane {})", online.bold(), pane.dimmed());
    }
    Ok(())
}
