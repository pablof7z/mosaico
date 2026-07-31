//! Operator channel manager TUI — navigate, edit about, delete (kind:9008).
//!
//! Shown only from a non-agent interactive session via `mosaico channel list`
//! with no list flags. Same visual taste as the operator home picker.

mod data;
mod picker;
mod render;
mod state;

use anyhow::Result;
use std::io::IsTerminal;

/// Open the channel manager when stdin/stdout are terminals.
pub(in crate::cli) async fn run() -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        anyhow::bail!(
            "channel manager is interactive — run `mosaico channel list` in a terminal, \
             or pass -a / -r / --workspace for a text listing"
        );
    }
    picker::run().await
}
