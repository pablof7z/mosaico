mod args;
mod data;
mod delete;
mod editor;

use anyhow::{bail, Result};
use owo_colors::OwoColorize as _;
use std::io::IsTerminal as _;

use args::AgentAction;
pub(in crate::cli) use args::AgentsArgs;
use data::{AgentKind, AgentRow};

pub(in crate::cli) async fn agents(args: AgentsArgs) -> Result<()> {
    match args.action {
        Some(AgentAction::List) => list(),
        Some(AgentAction::Add {
            slug,
            harness,
            profile,
        }) => add(&slug, &harness, profile.as_deref()).await,
        Some(AgentAction::Remove { slug }) => remove(&slug).await,
        None => interactive().await,
    }
}

async fn interactive() -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("mosaico agents is interactive — run it in a terminal");
    }
    loop {
        let rows = data::load()?;
        if rows.is_empty() {
            println!("No configured or installed agents.");
            return Ok(());
        }
        let picker_rows = rows.iter().map(picker_row).collect();
        match crate::cli::interactive::agent_picker::select(
            picker_rows,
            crate::cli::interactive::agent_picker::PickerMode::Manage,
        )? {
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
                publish_roster(None).await;
            }
            crate::cli::interactive::agent_picker::PickerAction::Delete(index) => {
                delete::delete(&rows[index]).await?;
            }
            crate::cli::interactive::agent_picker::PickerAction::Cancel => return Ok(()),
        }
    }
}

fn picker_row(row: &AgentRow) -> crate::cli::interactive::agent_picker::AgentPickerRow {
    let key = match row.per_session_key {
        Some(true) => "per-session",
        Some(false) => "persistent",
        None => "not configured",
    };
    let transport = row
        .transport
        .map(|value| value.as_str())
        .unwrap_or("choose transport");
    let bundle = row.bundle.as_deref().unwrap_or("new configuration");
    crate::cli::interactive::agent_picker::AgentPickerRow {
        name: row.slug.clone(),
        description: row.description.clone(),
        description_harness: None,
        provenance: Some(crate::cli::interactive::agent_picker::AgentProvenance {
            label: format!(
                "{} · {transport} · {bundle} · {key}",
                data::harness_name(row.harness)
            ),
            harness: row.harness,
        }),
    }
}

fn list() -> Result<()> {
    let rows = data::load()?;
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
            "{}  {}  {} · {}",
            row.slug.bold(),
            row.description,
            data::harness_name(row.harness).dimmed(),
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
    publish_roster(None).await;
    Ok(())
}

async fn remove(slug: &str) -> Result<()> {
    if crate::identity::remove_local_agent(&crate::config::mosaico_home(), slug)? {
        println!("Deleted {}", slug.bold());
        publish_roster(Some(slug)).await;
    } else {
        eprintln!("no such configured agent: {slug}");
    }
    Ok(())
}

pub(super) async fn publish_roster(remove_slug: Option<&str>) {
    match crate::cli::daemon_call_async(
        "agent_roster_publish",
        serde_json::json!({ "remove_slug": remove_slug }),
    )
    .await
    {
        Ok(value) => println!(
            "  roster publish: {} advertised, {} removed, {} failed",
            value["published"].as_u64().unwrap_or(0),
            value["removed"].as_u64().unwrap_or(0),
            value["failed"].as_array().map(Vec::len).unwrap_or(0)
        ),
        Err(error) => eprintln!("  roster publish deferred: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn management_row_names_actual_harness_last() {
        let row = AgentRow {
            slug: "reviewer".into(),
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
        assert_eq!(
            picker.provenance.unwrap().label,
            "Claude · acp · claude-acp · per-session"
        );
    }
}
