//! Commit onboarding decisions: write `config.json` and apply install mechanics.
//!
//! Runs after the TUI has torn down, so it emits ordinary styled stdout rather
//! than fighting the alternate screen. Mosaico does not run a relay; it is
//! pointed at an externally operated NIP-29 relay URL.

use anyhow::Result;
use owo_colors::OwoColorize;
use std::time::Duration;

use super::super::args::InstallOpts;
use super::super::device_config::{self, OnboardingIdentity};
use super::super::selection::InstallSelection;
use super::model::{Onboarding, RelayChoice};
use super::relay::{self, Probe};

const MANUAL_POLL_TIMEOUT: Duration = Duration::from_secs(90);
const POLL_INTERVAL: Duration = Duration::from_millis(1500);

pub(super) async fn commit(state: Onboarding) -> Result<()> {
    let relay_url = state.relay_url.trim().to_string();
    let manual = matches!(state.relay_choice(), RelayChoice::Manual);

    let identity = OnboardingIdentity {
        device_name: state.device_name.trim().to_string(),
        operator_pubkey_hex: state.identity.pubkey_hex.clone(),
        operator_nsec: state.identity.nsec.clone(),
    };

    println!("\n{}", "◆ Applying Mosaico setup".cyan().bold());
    device_config::write_onboarding(&identity, vec![relay_url.clone()])?;

    let selected_ids: Vec<&'static str> = state
        .all
        .iter()
        .zip(&state.selected)
        .filter(|(_, on)| **on)
        .map(|(h, _)| h.id)
        .collect();
    let selection = InstallSelection {
        skill: true,
        harnesses: state
            .all
            .iter()
            .filter(|h| selected_ids.contains(&h.id))
            .collect(),
    };

    let opts = InstallOpts::default();
    super::super::apply_install(&state.all, &selection, &opts).await?;

    if manual {
        wait_for_relay(&relay_url).await;
    }

    println!(
        "\n{}",
        "✓ Mosaico is ready. Restart open harness sessions, then run `mosaico doctor`."
            .green()
            .bold()
    );
    Ok(())
}

async fn wait_for_relay(url: &str) {
    println!("\n{}", "Bring up your relay".bold());
    println!(
        "  Provision a NIP-29 relay (Croissant is a good choice) reachable at {}.",
        url.cyan()
    );
    print!("  Waiting for it to come online");
    use std::io::Write as _;
    let _ = std::io::stdout().flush();

    let deadline = tokio::time::Instant::now() + MANUAL_POLL_TIMEOUT;
    loop {
        if let Probe::Usable = relay::probe(url).await {
            println!("\n  {}", "✓ relay online and NIP-29 ready".green());
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            println!(
                "\n  {}",
                "still offline — start it later; `mosaico doctor` will confirm.".yellow()
            );
            return;
        }
        print!(".");
        let _ = std::io::stdout().flush();
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}
