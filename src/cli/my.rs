use anyhow::{Context, Result};
use clap::Subcommand;

use super::session::SessionAction;

#[derive(Subcommand)]
pub(super) enum MyAction {
    /// Inspect or manage the current local session.
    Session {
        #[command(subcommand)]
        action: Option<SessionAction>,
    },
}

pub(super) fn my(action: MyAction) -> Result<()> {
    match action {
        MyAction::Session {
            action: Some(action),
        } => super::session::session(action),
        MyAction::Session { action: None } => briefing(),
    }
}

fn briefing() -> Result<()> {
    let value =
        crate::daemon::blocking::call("my_session", crate::cli::rpc_params(serde_json::json!({})))?;
    let fabric = value["fabric"]
        .as_str()
        .context("my session response missing fabric briefing")?;
    println!("{fabric}");
    Ok(())
}
