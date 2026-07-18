mod args;
mod data;
mod delete;
mod editor;
mod usage;

use anyhow::{Context, Result};
use owo_colors::OwoColorize as _;
use std::io::IsTerminal as _;

use args::AgentAction;
pub(in crate::cli) use args::AgentsArgs;
use data::{AgentKind, AgentRow};

pub(in crate::cli) async fn agents(args: AgentsArgs) -> Result<()> {
    if args.action.is_none() {
        return match args.launch_request()? {
            Some(request) => crate::cli::launch_cli::verbs::launch(request).await,
            None => interactive().await,
        };
    }
    match args.action.expect("checked above") {
        AgentAction::List => list().await,
        AgentAction::Add {
            slug,
            harness,
            profile,
        } => add(&slug, &harness, profile.as_deref()).await,
        AgentAction::Remove { slug } => remove(&slug).await,
    }
}

async fn interactive() -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return list().await;
    }
    let mut cursor = 0usize;
    loop {
        let mut rows =
            usage::ordered_rows(data::load()?, &usage::fetch(crate::util::now_secs()).await?);
        if rows.is_empty() {
            println!("No configured or installed agents.");
            return Ok(());
        }
        // Native-profile-only agents (no mosaico agent.json yet) sort after
        // everything else; `render::draw` uses this same grouping to draw a
        // separating blank line. `sort_by_key` is stable, so ordering within
        // each group is otherwise untouched.
        rows.sort_by_key(|row| row.kind == AgentKind::NativeProfile);
        let picker_rows = rows.iter().map(picker_row).collect();
        match crate::cli::interactive::agent_picker::select(picker_rows, cursor)? {
            crate::cli::interactive::agent_picker::PickerAction::Launch(index) => {
                let row = &rows[index];
                return crate::cli::launch_cli::verbs::launch(
                    crate::cli::launch_cli::LaunchRequest {
                        agent: row.slug.clone(),
                        root: None,
                        channel: None,
                        session_name: None,
                        prompt: None,
                    },
                )
                .await;
            }
            crate::cli::interactive::agent_picker::PickerAction::Edit(index) => {
                cursor = index;
                editor::edit(&rows[index]).await?;
                schedule_roster_refresh(None).await;
            }
            crate::cli::interactive::agent_picker::PickerAction::Delete(plan) => {
                cursor = plan.iter().map(|(index, _)| *index).min().unwrap_or(0);
                for (index, scope) in plan {
                    delete::delete(&rows[index], scope).await?;
                }
            }
            crate::cli::interactive::agent_picker::PickerAction::Cancel => return Ok(()),
        }
    }
}

fn picker_row(row: &AgentRow) -> crate::cli::interactive::agent_picker::AgentPickerRow {
    crate::cli::interactive::agent_picker::AgentPickerRow {
        name: row.slug.clone(),
        description: row.description.clone(),
        status: Some(crate::cli::interactive::agent_picker::AgentProvenance {
            label: data::harness_name(row.harness).to_string(),
            harness: row.harness,
        }),
        has_configured: row.kind == AgentKind::Configured,
        has_native_profile: row.native_profile.is_some(),
    }
}

async fn list() -> Result<()> {
    let rows = usage::ordered_rows(data::load()?, &usage::fetch(crate::util::now_secs()).await?);
    if rows.is_empty() {
        println!("No configured or installed agents.");
        return Ok(());
    }
    for row in rows {
        let source = match row.kind {
            AgentKind::Configured => "configured",
            AgentKind::NativeProfile => "native profile",
            AgentKind::Generic => "generic",
        };
        println!(
            "{}  {} · {}",
            row.slug.bold(),
            row.description,
            source.dimmed()
        );
    }
    Ok(())
}

async fn add(slug: &str, harness: &str, profile: Option<&str>) -> Result<()> {
    let saved = save_agent_config(slug, harness, profile.map(str::to_string), None).await?;
    println!(
        "{} {} · {}",
        if saved.created { "Created" } else { "Updated" },
        slug.bold(),
        saved.harness
    );
    schedule_roster_refresh(None).await;
    Ok(())
}

async fn remove(slug: &str) -> Result<()> {
    if remove_agent_config(slug).await? {
        println!("Deleted {}", slug.bold());
        schedule_roster_refresh(Some(slug)).await;
    } else {
        eprintln!("no such configured agent: {slug}");
    }
    Ok(())
}

pub(super) struct SavedAgent {
    pub(super) created: bool,
    pub(super) harness: String,
}

pub(super) async fn save_agent_config(
    slug: &str,
    harness: &str,
    profile: Option<String>,
    per_session_key: Option<bool>,
) -> Result<SavedAgent> {
    let value = crate::cli::daemon_call_async(
        "agent_save",
        serde_json::json!({
            "slug": slug,
            "harness": harness,
            "profile": profile,
            "per_session_key": per_session_key,
        }),
    )
    .await?;
    Ok(SavedAgent {
        created: value["created"]
            .as_bool()
            .context("agent_save response missing created")?,
        harness: value["harness"]
            .as_str()
            .context("agent_save response missing harness")?
            .to_string(),
    })
}

pub(super) async fn remove_agent_config(slug: &str) -> Result<bool> {
    let value =
        crate::cli::daemon_call_async("agent_remove", serde_json::json!({ "slug": slug })).await?;
    value["removed"]
        .as_bool()
        .context("agent_remove response missing removed")
}

pub(super) async fn schedule_roster_refresh(remove_slug: Option<&str>) {
    match crate::cli::daemon_call_async(
        "agent_roster_refresh",
        serde_json::json!({ "remove_slug": remove_slug }),
    )
    .await
    {
        Ok(_) => {}
        Err(error) => eprintln!("  roster refresh deferred: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn management_row_keeps_harness_in_status_only() {
        let row = AgentRow {
            slug: "reviewer".into(),
            agent_slug: "reviewer".into(),
            description: "Reviews changes".into(),
            harness: crate::session::Harness::ClaudeCode,
            bundle: Some("claude-acp".into()),
            transport: Some(crate::harness::Transport::Acp),
            profile: None,
            per_session_key: Some(true),
            kind: AgentKind::Configured,
            native_profile: None,
        };
        let picker = picker_row(&row);
        assert_eq!(picker.description, "Reviews changes");
        assert_eq!(picker.status.unwrap().label, "Claude");
    }
}
