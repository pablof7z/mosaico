mod args;
mod data;
mod delete;
mod editor;
mod usage;

use anyhow::Result;
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
    loop {
        let rows =
            usage::ordered_rows(data::load()?, &usage::fetch(crate::util::now_secs()).await?);
        if rows.is_empty() {
            println!("No configured or installed agents.");
            return Ok(());
        }
        let picker_rows = rows.iter().map(picker_row).collect();
        match crate::cli::interactive::agent_picker::select(picker_rows)? {
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
                editor::edit(&rows[index])?;
            }
            crate::cli::interactive::agent_picker::PickerAction::Delete(index) => {
                delete::delete(&rows[index]).await?;
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
    let (identity, created) = crate::identity::add_local_agent(
        &crate::config::mosaico_home(),
        slug,
        harness,
        profile,
        crate::util::now_secs(),
    )?;
    println!(
        "{} {} · {}",
        if created { "Created" } else { "Updated" },
        slug.bold(),
        identity.harness
    );
    schedule_roster_refresh(None).await;
    Ok(())
}

async fn remove(slug: &str) -> Result<()> {
    if crate::identity::remove_local_agent(&crate::config::mosaico_home(), slug)? {
        println!("Deleted {}", slug.bold());
        schedule_roster_refresh(Some(slug)).await;
    } else {
        eprintln!("no such configured agent: {slug}");
    }
    Ok(())
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
