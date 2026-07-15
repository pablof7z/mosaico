use super::*;

// ── agent (local keystore) ────────────────────────────────────────────────────

pub async fn agent(action: AgentAction) -> Result<()> {
    let mosaico_home = crate::config::mosaico_home();
    match action {
        AgentAction::List => {
            let rows = crate::identity::list_local_agent_details(&mosaico_home);
            if rows.is_empty() {
                println!(
                    "No local agents in {}",
                    mosaico_home.join("agents").display()
                );
                println!("Add one with: mosaico mgmt agent add <slug> --harness <bundle>");
                return Ok(());
            }
            let max_slug = rows.iter().map(|r| r.slug.len()).max().unwrap_or(0);
            for r in &rows {
                let profile = r.profile.as_deref().unwrap_or("default");
                println!(
                    "{:<max_slug$}  {}  profile={}",
                    r.slug.bold(),
                    r.harness.dimmed(),
                    profile.dimmed()
                );
            }
        }
        AgentAction::Add {
            slug,
            workspaces,
            harness,
            profile,
        } => {
            let (id, created) = crate::identity::add_local_agent(
                &mosaico_home,
                &slug,
                &harness,
                profile.as_deref(),
                now_secs(),
            )?;
            let verb = if created { "created" } else { "updated" };
            println!(
                "{} {} {}",
                verb,
                slug.bold(),
                pubkey_short(&id.pubkey_hex()).cyan()
            );
            println!("  harness: {}", id.harness.dimmed());
            println!(
                "  profile: {}",
                id.profile.as_deref().unwrap_or("default").dimmed()
            );
            if created {
                if let Some(byline) = prompt_use_criteria()? {
                    crate::identity::set_local_agent_byline(&mosaico_home, &slug, Some(byline))?;
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
            let found = crate::identity::list_local_agent_details(&mosaico_home)
                .into_iter()
                .any(|a| a.slug == slug);
            if !found {
                anyhow::bail!(
                    "no such local agent: {slug} (add it with `mosaico mgmt agent add {slug}`)"
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
            match crate::identity::remove_local_agent(&mosaico_home, &slug)? {
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
