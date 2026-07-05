//! Interactive "Models" screen: assign a model to a role by fuzzy-searching
//! the *live* model list from whichever provider you pick — Ollama's local
//! `/api/tags`, OpenRouter's public catalog — or typing one in by hand for
//! anything else (`claude-cli` and unrecognized custom providers).

use super::catalog::{self, CatalogModel};
use super::store::{LlmsFile, ProvidersFile};
use super::util::prompted;
use anyhow::Result;
use inquire::{Confirm, Select, Text};
use owo_colors::OwoColorize;

/// Roles actually read anywhere in tenex-edge (via
/// `crate::llmconfig::resolve_role`) — keep this in sync with call sites
/// (currently just `distill.rs`'s `edge-distillation` lookup). There's no
/// free-text "new role" option: a role name the code never resolves does
/// nothing, so inventing one is a dead end, not a feature.
const KNOWN_ROLES: &[&str] = &["edge-distillation"];
const CLAUDE_CLI_KEY: &str = "claude-cli";
const CLAUDE_CLI_MODEL_SUGGESTIONS: &[&str] = &[
    "claude-sonnet-5",
    "claude-opus-4-8",
    "claude-haiku-4-5-20251001",
    "claude-fable-5",
];
const TYPE_MANUALLY: &str = "Type a model id manually...";

pub(super) async fn run() -> Result<()> {
    loop {
        let llms = LlmsFile::load()?;
        let providers = ProvidersFile::load()?;
        print_roles(&llms);

        let Some(role) = pick_role(&llms)? else {
            return Ok(());
        };
        let Some(provider) = pick_provider(&providers).await? else {
            continue;
        };
        let Some(model) = pick_model(&provider, &providers).await? else {
            continue;
        };

        let Some(true) = prompted(
            Confirm::new(&format!("Set role \"{role}\" -> {provider}/{model}?"))
                .with_default(true)
                .prompt(),
        )?
        else {
            continue;
        };

        let mut llms = LlmsFile::load()?;
        let config_name = llms.set_role(&role, &provider, &model);
        llms.save()?;
        println!("  {} {role} now resolves to {config_name}", "✓".green());
    }
}

fn print_roles(llms: &LlmsFile) {
    println!();
    let roles = llms.roles();
    if roles.is_empty() {
        println!("  {}", "no roles configured yet".dimmed());
    } else {
        for (role, config_name) in &roles {
            let detail = match llms.configuration(config_name) {
                Some((provider, model)) => format!("{provider} / {model}"),
                None => config_name.clone(),
            };
            println!("  {role:<20} {}", detail.dimmed());
        }
    }
    println!();
}

/// Offers only roles tenex-edge's own code resolves (`KNOWN_ROLES`), plus
/// any already present in `llms.json` that aren't in that list (e.g. a role
/// used by a sibling TENEX host sharing this file) — never a free-text
/// entry, since an unresolved role name is inert.
fn pick_role(llms: &LlmsFile) -> Result<Option<String>> {
    let mut choices: Vec<String> = KNOWN_ROLES.iter().map(|r| r.to_string()).collect();
    for (role, _) in llms.roles() {
        if !choices.contains(&role) {
            choices.push(role);
        }
    }

    prompted(
        Select::new("Which role?", choices)
            .with_help_message("only roles tenex-edge's own code resolves are offered")
            .prompt(),
    )
}

/// Providers eligible to serve a role: everything with a `providers.json`
/// entry, plus `claude-cli`, which needs none (the CLI handles its own
/// auth). If none are configured at all, offer to jump into the Providers
/// "Add" flow right here instead of dead-ending.
async fn pick_provider(providers: &ProvidersFile) -> Result<Option<String>> {
    let configured: Vec<String> = providers.list().into_iter().map(|(name, _)| name).collect();

    let mut choices = configured.clone();
    if !choices.iter().any(|c| c == CLAUDE_CLI_KEY) {
        choices.push(CLAUDE_CLI_KEY.to_string());
    }

    if configured.is_empty() {
        println!(
            "  {}",
            "no providers configured — Models needs at least one to fetch from".yellow()
        );
        let Some(true) = prompted(
            Confirm::new("Set up a provider now?")
                .with_default(true)
                .prompt(),
        )?
        else {
            return Ok(None);
        };
        Box::pin(super::providers_menu::run()).await?;
        return Ok(None);
    }

    if choices.len() == 1 {
        let only = choices.into_iter().next().unwrap();
        println!(
            "  {}",
            format!("using {only} (only configured provider)").dimmed()
        );
        return Ok(Some(only));
    }

    prompted(Select::new("Which provider?", choices).prompt())
}

async fn pick_model(provider: &str, providers: &ProvidersFile) -> Result<Option<String>> {
    match provider {
        "ollama" => {
            let Some(base_url) = providers.get("ollama") else {
                println!("  {}", "ollama has no base URL configured".yellow());
                return Ok(None);
            };
            println!(
                "  {}",
                format!("fetching models from {base_url}...").dimmed()
            );
            fetch_and_pick(catalog::ollama_models(&base_url).await, "ollama")
        }
        "openrouter" => {
            let api_key = providers.get("openrouter").unwrap_or_default();
            println!("  {}", "fetching models from openrouter.ai...".dimmed());
            fetch_and_pick(catalog::openrouter_models(&api_key).await, "openrouter")
        }
        CLAUDE_CLI_KEY => pick_with_suggestions(CLAUDE_CLI_MODEL_SUGGESTIONS),
        _ => prompted(Text::new(&format!("{provider} model id")).prompt()),
    }
}

fn fetch_and_pick(
    result: anyhow::Result<Vec<CatalogModel>>,
    provider: &str,
) -> Result<Option<String>> {
    let models = match result {
        Ok(models) if models.is_empty() => {
            println!("  {}", format!("{provider} returned no models").yellow());
            return Ok(None);
        }
        Ok(models) => models,
        Err(e) => {
            println!("  {} {e:#}", "✗".red());
            return Ok(None);
        }
    };

    let picked = prompted(
        Select::new("Which model?", models)
            .with_help_message("type to fuzzy-search")
            .with_page_size(12)
            .prompt(),
    )?;
    Ok(picked.map(|m| m.id))
}

fn pick_with_suggestions(suggestions: &[&str]) -> Result<Option<String>> {
    let mut choices: Vec<String> = suggestions.iter().map(|s| s.to_string()).collect();
    choices.push(TYPE_MANUALLY.to_string());

    let Some(choice) = prompted(Select::new("Which model?", choices).prompt())? else {
        return Ok(None);
    };

    if choice != TYPE_MANUALLY {
        return Ok(Some(choice));
    }
    prompted(Text::new("Model id").prompt())
}
