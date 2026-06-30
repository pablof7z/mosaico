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
                let cmd = r
                    .command
                    .as_ref()
                    .map(|c| c.join(" "))
                    .unwrap_or_else(|| "(default harness)".to_string());
                println!(
                    "{:<max_slug$}  {}  {}",
                    r.slug.bold(),
                    pubkey_short(&r.pubkey).cyan(),
                    cmd.dimmed()
                );
            }
        }
        AgentAction::Add {
            slug,
            projects,
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
            match &id.command {
                Some(c) => println!("  spawns: {}", c.join(" ").dimmed()),
                None => println!("  spawns: {}", "(default harness)".dimmed()),
            }
            // Publish the kind:0 identity card so the agent is discoverable on the
            // indexer relay immediately, not just after its first session. Best
            // effort: the keypair already exists locally and the session engine
            // re-publishes the same Profile on first run, so a publish failure
            // (e.g. daemon/relay down) must not fail the create.
            if created {
                publish_profile(&slug).await;
            }
            assign_to_projects(&id.pubkey_hex(), &projects).await?;
        }
        AgentAction::Assign { slug, projects } => {
            let pubkey = crate::identity::list_local_agent_details(&edge_home)
                .into_iter()
                .find(|a| a.slug == slug)
                .map(|a| a.pubkey)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no such local agent: {slug} (add it with `tenex-edge agent add {slug}`)"
                    )
                })?;
            println!("{} {}", slug.bold(), pubkey_short(&pubkey).cyan());
            assign_to_projects(&pubkey, &projects).await?;
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
        }
    }
    Ok(())
}

/// `tenex-edge agents` — the invitable roster: every local-keystore agent with
/// its "when to use" byline, plus the gesture to pull one in. Distinct from
/// `agent list` (a keystore-management view of pubkeys/commands): this is the
/// recruiting screen an agent or human consults before an `invite`. Reads the
/// local store directly — no daemon round-trip.
pub async fn agents_roster() -> Result<()> {
    let edge_home = crate::config::edge_home();
    let rows = crate::identity::list_local_agents(&edge_home);
    if rows.is_empty() {
        println!(
            "No agents to invite (none in {}).",
            edge_home.join("agents").display()
        );
        println!("Add one with: tenex-edge agent add <slug> [-- <command>]");
        return Ok(());
    }
    println!("Agents you can invite (spawns a fresh session in your current channel):");
    for (slug, _command, _agent_def, byline) in &rows {
        match byline.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
            Some(b) => println!("  @{} — {}", slug.bold(), b),
            None => println!("  @{}", slug.bold()),
        }
    }
    println!("\nInvite one with: tenex-edge invite <slug>");
    Ok(())
}

/// Publish an agent's kind:0 identity card via the daemon (which owns the
/// transport pool including the indexer relay). Best effort: failures are
/// reported but never abort agent creation.
async fn publish_profile(slug: &str) {
    match daemon_call_async("publish_profile", serde_json::json!({ "slug": slug })).await {
        Ok(v) => {
            let event_id = v["event_id"].as_str().unwrap_or("?");
            let short = &event_id[..event_id.len().min(8)];
            println!("  published profile (kind:0) {}", short.dimmed());
        }
        Err(e) => eprintln!("  profile publish deferred to first session: {e}"),
    }
}

/// Add `pubkey` to each project's NIP-29 group via the daemon's `project_add`
/// RPC. Per-project failures (e.g. operator not a group admin) are reported as
/// they happen so the remaining assignments still run, but any failure makes the
/// whole command exit non-zero — a partially-applied grant must not look like a
/// clean success to a caller (or a script) that checks the exit code.
async fn assign_to_projects(pubkey: &str, projects: &[String]) -> Result<()> {
    let mut failures = 0usize;
    for project in projects {
        match daemon_call_async(
            "project_add",
            serde_json::json!({ "project": project, "pubkey": pubkey }),
        )
        .await
        {
            Ok(_) => println!("  assigned to {}", project.bold()),
            Err(e) => {
                eprintln!("  failed to assign to {}: {}", project.bold(), e);
                failures += 1;
            }
        }
    }
    if failures > 0 {
        anyhow::bail!(
            "{failures} of {} project assignment(s) failed",
            projects.len()
        );
    }
    Ok(())
}
