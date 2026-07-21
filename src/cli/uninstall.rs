//! Respectful top-level uninstallation.

use anyhow::{bail, Context, Result};
use clap::Args;
use dialoguer::Confirm;
use std::io::{self, IsTerminal as _};
use std::path::{Path, PathBuf};

#[derive(Args)]
pub(super) struct UninstallArgs {
    /// Show every removal without changing files or stopping processes.
    #[arg(long)]
    dry_run: bool,
    /// Also remove device identity, trust, sessions, logs, and relay data.
    #[arg(long)]
    purge_state: bool,
    /// Confirm state removal in a non-interactive shell.
    #[arg(long, requires = "purge_state")]
    yes: bool,
}

pub(super) async fn uninstall(args: UninstallArgs) -> Result<()> {
    super::install::uninstall_everywhere(args.dry_run).await?;
    super::local_relay::stop(args.dry_run)?;
    if args.dry_run {
        println!("would stop the Mosaico daemon without signaling detached PTY supervisors");
    } else {
        super::daemon_lifecycle::stop()?;
    }

    let home = crate::config::mosaico_home();
    let remove_state = choose_state_removal(&args, &home)?;
    if remove_state {
        if args.dry_run {
            println!("would remove local Mosaico state at {}", home.display());
        } else {
            remove_state_home(&home)?;
            println!("removed local Mosaico state at {}", home.display());
            println!(
                "The Mosaico executable remains installed; remove it with its package manager."
            );
        }
    } else {
        println!("preserved local Mosaico state at {}", home.display());
        println!("This keeps device identity, operator trust, sessions, logs, and relay data.");
    }
    Ok(())
}

fn choose_state_removal(args: &UninstallArgs, home: &Path) -> Result<bool> {
    if args.dry_run {
        return Ok(args.purge_state);
    }
    if args.yes {
        validate_state_home(home)?;
        return Ok(true);
    }
    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        if args.purge_state {
            bail!(
                "state removal needs explicit confirmation in a non-interactive shell; add --yes"
            );
        }
        return Ok(false);
    }

    println!("\nLocal state is separate from harness integrations.");
    println!("Path: {}", home.display());
    println!("It contains device identity, operator trust, session history, logs, and relay data.");
    println!("Removing it is not recoverable.");
    let confirmed = Confirm::new()
        .with_prompt("Also remove this local Mosaico state?")
        .default(false)
        .interact()?;
    if confirmed {
        validate_state_home(home)?;
    }
    Ok(confirmed)
}

fn validate_state_home(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        bail!(
            "refusing to remove non-absolute MOSAICO_HOME {}",
            path.display()
        );
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let cwd = std::env::current_dir().context("resolving current directory")?;
    let normal_components = path
        .components()
        .filter(|component| matches!(component, std::path::Component::Normal(_)))
        .count();
    let broad_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            matches!(
                name,
                "tmp" | "var" | "opt" | "usr" | "etc" | "home" | "Users" | "private" | "Volumes"
            )
        });
    if path == Path::new("/")
        || normal_components < 2
        || broad_name
        || home
            .as_deref()
            .is_some_and(|home| path == home || home.starts_with(path))
        || cwd == path
        || cwd.starts_with(path)
    {
        bail!("refusing to remove unsafe MOSAICO_HOME {}", path.display());
    }
    if path.is_dir()
        && path.file_name().and_then(|name| name.to_str()) != Some(".mosaico")
        && !["config.json", "state.db", "nmp.redb", "daemon.log", "relay"]
            .into_iter()
            .any(|marker| path.join(marker).exists())
    {
        bail!(
            "refusing to remove {} because it has no recognizable Mosaico state",
            path.display()
        );
    }
    Ok(())
}

fn remove_state_home(path: &Path) -> Result<()> {
    validate_state_home(path)?;
    let metadata = match path.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    if metadata.file_type().is_symlink() || metadata.is_file() {
        std::fs::remove_file(path).with_context(|| format!("removing {}", path.display()))
    } else {
        std::fs::remove_dir_all(path).with_context(|| format!("removing {}", path.display()))
    }
}

#[cfg(test)]
#[path = "uninstall/tests.rs"]
mod tests;
