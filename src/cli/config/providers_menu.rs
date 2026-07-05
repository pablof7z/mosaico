//! Interactive "Providers" screen: add/edit/remove entries in
//! `providers.json` (OpenRouter's API key, Ollama's base URL, ...).
//!
//! The provider list and its actions are one flow: picking a configured
//! provider drops straight into Edit/Test/Remove for it, rather than making
//! the user pick "Edit" first and then which provider.

use super::catalog;
use super::store::{LlmsFile, ProvidersFile};
use super::util::{mask_secret, prompted};
use anyhow::Result;
use inquire::{Confirm, Password, PasswordDisplayMode, Select, Text};
use owo_colors::OwoColorize;

/// `(key, display label, value is a URL rather than a secret)`.
const KNOWN: &[(&str, &str, bool)] = &[
    ("openrouter", "OpenRouter", false),
    ("ollama", "Ollama", true),
    ("anthropic", "Anthropic", false),
];

const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
const ADD_PROVIDER: &str = "+ Add provider";
const BACK: &str = "← Back";
const OTHER: &str = "Other...";

pub(super) async fn run() -> Result<()> {
    loop {
        let file = ProvidersFile::load()?;
        let entries = file.list();

        let mut items: Vec<String> = entries
            .iter()
            .map(|(name, value)| row_label(name, value))
            .collect();
        items.push(ADD_PROVIDER.to_string());
        items.push(BACK.to_string());

        let Some(choice) = prompted(
            Select::new("Providers", items.clone())
                .with_help_message("configure API keys and endpoints used to resolve models")
                .prompt(),
        )?
        else {
            return Ok(());
        };

        if choice == BACK {
            return Ok(());
        }
        if choice == ADD_PROVIDER {
            add(&entries).await?;
            continue;
        }

        let idx = items.iter().position(|i| *i == choice).unwrap_or(0);
        if let Some((key, _)) = entries.get(idx) {
            provider_actions(&file, key).await?;
        }
    }
}

fn row_label(name: &str, value: &str) -> String {
    let label = known_label(name).unwrap_or(name);
    if is_url_provider(name) {
        format!("{label:<12} {value}")
    } else {
        format!("{label:<12} {}", mask_secret(value))
    }
}

fn known_label(key: &str) -> Option<&'static str> {
    KNOWN.iter().find(|(k, _, _)| *k == key).map(|(_, l, _)| *l)
}

fn is_url_provider(key: &str) -> bool {
    KNOWN
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(_, _, is_url)| *is_url)
        .unwrap_or(false)
}

async fn add(existing: &[(String, String)]) -> Result<()> {
    let mut choices: Vec<&str> = KNOWN
        .iter()
        .filter(|(key, _, _)| !existing.iter().any(|(name, _)| name == key))
        .map(|(_, label, _)| *label)
        .collect();
    choices.push(OTHER);

    let Some(choice) = prompted(Select::new("Add which provider?", choices).prompt())? else {
        return Ok(());
    };

    let key = if choice == OTHER {
        let help = "lowercase slug, becomes the key in providers.json, e.g. \"my-proxy\"";
        let Some(name) = prompted(Text::new("Provider name").with_help_message(help).prompt())?
        else {
            return Ok(());
        };
        name.trim().to_lowercase()
    } else {
        KNOWN
            .iter()
            .find(|(_, label, _)| *label == choice)
            .map(|(key, _, _)| key.to_string())
            .unwrap_or_default()
    };

    if key.is_empty() {
        println!("  {}", "provider name can't be empty — aborted".red());
        return Ok(());
    }
    if existing.iter().any(|(name, _)| name == &key) {
        let msg = format!("\"{key}\" already exists — pick it from the list to edit it");
        println!("  {}", msg.yellow());
        return Ok(());
    }

    let is_url = if choice == OTHER {
        prompted(
            Confirm::new("Is this a base URL (like Ollama) rather than an API key?")
                .with_default(false)
                .prompt(),
        )?
        .unwrap_or(false)
    } else {
        is_url_provider(&key)
    };

    let Some(value) = prompt_value(&key, is_url, None)? else {
        return Ok(());
    };

    let mut file = ProvidersFile::load()?;
    file.set(&key, &value);
    file.save()?;
    println!("  {} saved providers.json — {key} configured", "✓".green());

    if key == "ollama" || key == "openrouter" {
        let Some(true) = prompted(
            Confirm::new(&format!("Assign a model role using {key} now?"))
                .with_default(false)
                .prompt(),
        )?
        else {
            return Ok(());
        };
        super::models_menu::run().await?;
    }
    Ok(())
}

async fn provider_actions(file: &ProvidersFile, key: &str) -> Result<()> {
    let is_url = is_url_provider(key);
    let current = file.get(key);
    let header = match &current {
        Some(v) if is_url => format!("{key} — {v}"),
        Some(v) => format!("{key} — {}", mask_secret(v)),
        None => key.to_string(),
    };

    let edit_label = if is_url {
        "Edit base URL"
    } else {
        "Edit API key"
    };
    let mut choices = vec![edit_label];
    let testable = key == "ollama" || key == "openrouter";
    if testable {
        choices.push("Test connection");
    }
    choices.push("Remove provider");
    choices.push(BACK);

    let Some(choice) = prompted(Select::new(&header, choices).prompt())? else {
        return Ok(());
    };

    match choice {
        c if c == edit_label => edit_one(key, current.as_deref(), is_url)?,
        "Test connection" => test_connection(file, key).await,
        "Remove provider" => remove_one(key)?,
        _ => {}
    }
    Ok(())
}

fn edit_one(key: &str, current: Option<&str>, is_url: bool) -> Result<()> {
    let Some(value) = prompt_value(key, is_url, current)? else {
        return Ok(());
    };

    let mut file = ProvidersFile::load()?;
    file.set(key, &value);
    file.save()?;
    println!("  {} updated {key}", "✓".green());
    Ok(())
}

async fn test_connection(file: &ProvidersFile, key: &str) {
    println!("  {}", format!("testing {key}...").dimmed());
    let result = match key {
        "ollama" => match file.get("ollama") {
            Some(url) => catalog::ollama_models(&url).await,
            None => {
                println!("  {}", "ollama has no base URL configured".yellow());
                return;
            }
        },
        "openrouter" => catalog::openrouter_models(&file.get(key).unwrap_or_default()).await,
        _ => return,
    };

    match result {
        Ok(models) => println!("  {} reachable — {} models", "✓".green(), models.len()),
        Err(e) => println!("  {} {e:#}", "✗".red()),
    }
}

fn remove_one(key: &str) -> Result<()> {
    let llms = LlmsFile::load()?;
    let dependent: Vec<String> = llms
        .roles()
        .into_iter()
        .filter(|(_, config_name)| {
            llms.configuration(config_name)
                .map(|(provider, _)| provider == key)
                .unwrap_or(false)
        })
        .map(|(role, _)| role)
        .collect();

    if !dependent.is_empty() {
        println!(
            "  {}",
            format!(
                "{} role(s) use {key}: {} — they will stop working until reassigned.",
                dependent.len(),
                dependent.join(", ")
            )
            .yellow()
        );
    }

    let Some(true) = prompted(
        Confirm::new(&format!("Remove \"{key}\" from providers.json?"))
            .with_default(false)
            .prompt(),
    )?
    else {
        return Ok(());
    };

    let mut file = ProvidersFile::load()?;
    file.remove(key);
    file.save()?;
    println!("  {} removed {key} from providers.json", "✓".green());
    Ok(())
}

/// Prompt for a provider's value: a plain (visible) URL for Ollama-style
/// providers, or a masked secret otherwise. `current` prefills the URL case
/// (URLs aren't secrets — editing beats retyping) and is shown as a masked
/// hint above the prompt for the secret case, never used to prefill it.
fn prompt_value(key: &str, is_url: bool, current: Option<&str>) -> Result<Option<String>> {
    if is_url {
        let initial = current
            .map(str::to_string)
            .or_else(|| (key == "ollama").then(|| DEFAULT_OLLAMA_URL.to_string()));
        let message = format!("{key} base URL");
        let mut prompt = Text::new(&message);
        if let Some(initial) = &initial {
            prompt = prompt.with_initial_value(initial);
        }
        return prompted(prompt.prompt());
    }

    if let Some(current) = current {
        println!(
            "  {}",
            format!("current: {} — leave empty to keep", mask_secret(current)).dimmed()
        );
    }
    let message = format!("{key} API key");
    let value = prompted(
        Password::new(&message)
            .with_display_mode(PasswordDisplayMode::Masked)
            .without_confirmation()
            .prompt(),
    )?;
    match value {
        Some(v) if v.is_empty() => Ok(current.map(str::to_string)),
        other => Ok(other),
    }
}
