//! `tenex-edge config` — interactive setup for `providers.json` and
//! `llms.json` (the model-resolution files documented in
//! `crate::llmconfig`), built on the `inquire` prompt library.

mod args;
mod catalog;
mod models_menu;
mod providers_menu;
mod store;
mod theme;
mod util;

use anyhow::{bail, Result};
use args::ConfigAction;
pub use args::ConfigArgs;
use inquire::Select;
use std::io::IsTerminal;
use util::prompted;

pub async fn config(args: ConfigArgs) -> Result<()> {
    if !std::io::stdin().is_terminal() {
        bail!("tenex-edge config is interactive — run it in a terminal");
    }
    theme::install();
    match args.action {
        Some(ConfigAction::Providers) => providers_menu::run().await,
        Some(ConfigAction::Models) => models_menu::run().await,
        None => top_menu().await,
    }
}

async fn top_menu() -> Result<()> {
    loop {
        let choice = prompted(
            Select::new("tenex-edge config", vec!["Providers", "Models", "Quit"])
                .with_help_message(
                    "Providers: API keys/endpoints. Models: assign a model to a role.",
                )
                .prompt(),
        )?;

        match choice {
            Some("Providers") => providers_menu::run().await?,
            Some("Models") => models_menu::run().await?,
            _ => return Ok(()),
        }
    }
}
