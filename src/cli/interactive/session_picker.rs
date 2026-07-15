mod data;
mod layout;
mod picker;

use self::data::SessionRow;
use anyhow::{bail, Result};
use std::io::IsTerminal;

#[derive(Clone, Debug)]
struct SessionChoice {
    row: SessionRow,
}

pub(in crate::cli) async fn sessions() -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("mosaico sessions is interactive — run it in a terminal");
    }

    loop {
        let choices = data::fetch_sessions()
            .await?
            .into_iter()
            .map(|row| SessionChoice { row })
            .collect::<Vec<_>>();
        if choices.is_empty() {
            println!("No local sessions.");
            return Ok(());
        }

        let (_, terminal_rows) = crossterm::terminal::size().unwrap_or((100, 28));
        match picker::select(choices, terminal_rows)? {
            picker::PickerAction::Attach(choice) => {
                let Some(pty_id) = choice.row.pty_id else {
                    bail!("selected session no longer has an attachable endpoint");
                };
                return crate::pty::attach(&pty_id, &choice.row.handle);
            }
            picker::PickerAction::Kill(choice) => kill(choice).await?,
            picker::PickerAction::Cancel => return Ok(()),
        }
    }
}

async fn kill(choice: SessionChoice) -> Result<()> {
    let value = crate::cli::daemon_call_async(
        "session_kill",
        serde_json::json!({
            "session": choice.row.npub,
            "pty_id": choice.row.pty_id,
            "revoke_memberships": true,
        }),
    )
    .await?;
    if value["killed"].as_bool().unwrap_or(false) {
        if !value["cleanup_confirmed"].as_bool().unwrap_or(true) {
            let failures = value["cleanup_failures"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join("; ");
            bail!(
                "killed @{}, but immediate fabric cleanup was not confirmed: {failures}",
                choice.row.handle
            );
        }
        return Ok(());
    }
    bail!(
        "could not kill @{}: {}",
        choice.row.handle,
        value["reason"].as_str().unwrap_or("kill failed")
    )
}
