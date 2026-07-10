use super::*;

// ── agent (local keystore) ────────────────────────────────────────────────────

pub async fn agent(action: AgentAction) -> Result<()> {
    let edge_home = crate::config::edge_home();
    match action {
        AgentAction::List => {
            let rows = crate::identity::list_local_agent_details(&edge_home);
            if rows.is_empty() {
                println!("No local agents in {}", edge_home.join("agents").display());
                println!("Add one with: tenex-edge agent add <slug> [-- <command>]");
                return Ok(());
            }
            let max_slug = rows.iter().map(|r| r.slug.len()).max().unwrap_or(0);
            for r in &rows {
                let cmd = display_commands(&r.commands);
                println!("{:<max_slug$}  {}", r.slug.bold(), cmd.dimmed());
            }
        }
        AgentAction::Add {
            slug,
            workspaces,
            command_str,
            command,
        } => {
            let command = match command_str {
                Some(s) => Some(shlex::split(&s).unwrap_or_else(|| vec![s])),
                None if !command.is_empty() => Some(command),
                _ => None,
            };
            let (id, created) =
                crate::identity::add_local_agent(&edge_home, &slug, command, now_secs())?;
            let verb = if created { "created" } else { "updated" };
            println!(
                "{} {} {}",
                verb,
                slug.bold(),
                pubkey_short(&id.pubkey_hex()).cyan()
            );
            match id.commands.first() {
                Some(c) => println!("  spawns: {}", c.display().dimmed()),
                None => println!("  spawns: {}", "(no commands)".dimmed()),
            }
            if created {
                if let Some(byline) = prompt_use_criteria()? {
                    crate::identity::set_local_agent_byline(&edge_home, &slug, Some(byline))?;
                }
            }
            if !workspaces.is_empty() {
                println!(
                    "  workspace-specific assignment is not implemented; roster is advertised to every workspace"
                );
            }
            publish_roster(None).await;
        }
        AgentAction::Assign { slug, workspaces } => {
            let found = crate::identity::list_local_agent_details(&edge_home)
                .into_iter()
                .any(|a| a.slug == slug);
            if !found {
                anyhow::bail!(
                    "no such local agent: {slug} (add it with `tenex-edge agent add {slug}`)"
                );
            }
            if !workspaces.is_empty() {
                println!(
                    "workspace-specific assignment is not implemented; roster is advertised to every workspace"
                );
            }
            publish_roster(None).await;
        }
        AgentAction::Remove { slug } => {
            match crate::identity::remove_local_agent(&edge_home, &slug)? {
                Some(parked) => {
                    println!(
                        "removed {} (key parked at {})",
                        slug.bold(),
                        parked.display()
                    );
                }
                None => {
                    eprintln!("no such local agent: {slug}");
                }
            }
            publish_roster(Some(&slug)).await;
        }
    }
    Ok(())
}

fn display_commands(commands: &[crate::identity::LaunchCommand]) -> String {
    if commands.is_empty() {
        return "(no commands)".to_string();
    }
    commands
        .iter()
        .map(crate::identity::LaunchCommand::display)
        .collect::<Vec<_>>()
        .join(" | ")
}

fn prompt_use_criteria() -> Result<Option<String>> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Ok(None);
    }
    let criteria: String =
        dialoguer::Input::with_theme(&dialoguer::theme::ColorfulTheme::default())
            .with_prompt("Use criteria")
            .allow_empty(true)
            .interact_text()?;
    Ok(Some(criteria.trim().to_string()).filter(|s| !s.is_empty()))
}

/// `tenex-edge agents` — the advertised roster from materialized kind:30555
/// events. Distinct from `agent list` (a local keystore-management view of
/// commands): this is the recruiting screen an agent or human consults before an
/// `invite`.
pub async fn agents(action: Option<AgentsAction>) -> Result<()> {
    match action.unwrap_or(AgentsAction::List) {
        AgentsAction::List => agents_roster().await,
        AgentsAction::ListSessions { agent, since } => list_sessions(agent, since).await,
    }
}

async fn agents_roster() -> Result<()> {
    let v = daemon_call_async("agents_roster", serde_json::json!({})).await?;
    let rows = v["agents"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    if rows.is_empty() {
        println!("No agents advertised by 30555 roster events.");
        return Ok(());
    }
    println!("Available agents:");
    for row in rows {
        let agent = row["agent"].as_str().unwrap_or("?");
        let slug = row["slug"].as_str().unwrap_or(agent);
        let host = row["host"].as_str().unwrap_or("");
        let criteria = row["use_criteria"].as_str().unwrap_or("").trim();
        let channel = row["channel"].as_str().unwrap_or("");
        let invite_spec = if agent.contains('@') && !host.is_empty() {
            format!("{slug}@{host}")
        } else {
            slug.to_string()
        };
        match criteria.is_empty() {
            true => println!(
                "  {}  #{}  add: {}",
                agent.bold(),
                channel.dimmed(),
                invite_spec.dimmed()
            ),
            false => println!(
                "  {} — {}  #{}  add: {}",
                agent.bold(),
                criteria,
                channel.dimmed(),
                invite_spec.dimmed()
            ),
        }
    }
    println!("\nAdd one with: tenex-edge channel add --new-session <slug> <channel>");
    Ok(())
}

async fn list_sessions(agent: Option<String>, since: Option<String>) -> Result<()> {
    let since_ts = since.as_deref().map(parse_since).filter(|ts| *ts > 0);
    let v = daemon_call_async(
        "agents_list_sessions",
        serde_json::json!({ "agent": agent, "since": since_ts }),
    )
    .await?;
    let rows = v["sessions"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    if rows.is_empty() {
        println!("No prior sessions found.");
        return Ok(());
    }

    let now = now_secs();
    let mut current = String::new();
    for row in rows {
        let channel = row["channel"].as_str().unwrap_or("");
        if channel != current {
            current = channel.to_string();
            println!("#{}:", current);
        }
        let handle = row["handle"]
            .as_str()
            .or_else(|| row["agent"].as_str())
            .unwrap_or("?");
        let session_id = row["session_id"].as_str().unwrap_or("?");
        let title = row["title"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("(untitled)");
        let last_seen = row["last_seen"].as_u64().unwrap_or(0);
        let seen = if last_seen == 0 {
            "unknown".to_string()
        } else {
            relative_time(last_seen, now)
        };
        println!(
            "  * @{} [{}] - {} - last seen: {}",
            handle.bold(),
            session_id.dimmed(),
            title,
            seen
        );
    }
    Ok(())
}

async fn publish_roster(remove_slug: Option<&str>) {
    match daemon_call_async(
        "agent_roster_publish",
        serde_json::json!({ "remove_slug": remove_slug }),
    )
    .await
    {
        Ok(v) => {
            let published = v["published"].as_u64().unwrap_or(0);
            let removed = v["removed"].as_u64().unwrap_or(0);
            let failed = v["failed"].as_array().map(Vec::len).unwrap_or(0);
            println!(
                "  roster publish: {} advertised, {} removed, {} failed",
                published, removed, failed
            );
        }
        Err(e) => {
            eprintln!("  roster publish deferred: {e}");
        }
    }
}
