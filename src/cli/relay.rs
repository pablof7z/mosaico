use anyhow::{Context, Result};
use clap::Args;
use nostr_sdk::PublicKey;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const CROISSANT_REV: &str = env!("MOSAICO_CROISSANT_REV");
const CROISSANT_ARCHIVE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/croissant.zst"));

#[derive(Args)]
pub(super) struct RelayArgs {
    /// Interface on which the relay listens.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// TCP port on which the relay listens.
    #[arg(long, default_value_t = 9888)]
    port: u16,

    /// Public hostname used in relay URLs and NIP-98 authentication.
    #[arg(long)]
    domain: Option<String>,

    /// Persistent relay data directory.
    #[arg(long, value_name = "PATH")]
    data_dir: Option<PathBuf>,

    /// Relay owner public key as hex or npub; defaults to the first whitelisted operator.
    #[arg(long, value_name = "HEX_OR_NPUB")]
    owner_pubkey: Option<String>,
}

pub(super) fn relay(args: RelayArgs) -> Result<()> {
    let owner = resolve_owner(args.owner_pubkey)?;
    let executable = install_embedded()?;
    let data_dir = args
        .data_dir
        .unwrap_or_else(|| crate::config::mosaico_home().join("relay/data"));
    crate::config::ensure_dir(&data_dir)?;

    let endpoint = args
        .domain
        .as_ref()
        .map(|domain| format!("wss://{domain}"))
        .unwrap_or_else(|| format!("ws://{}:{}", args.host, args.port));
    eprintln!(
        "[mosaico] starting bundled Croissant {} at {endpoint}",
        &CROISSANT_REV[..12]
    );
    eprintln!("[mosaico] relay data: {}", data_dir.display());

    let mut command = Command::new(executable);
    command
        .env("HOST", args.host)
        .env("PORT", args.port.to_string())
        .env("DOMAIN", args.domain.unwrap_or_default())
        .env("DATAPATH", data_dir)
        .env("OWNER_PUBLIC_KEY", owner);
    replace_process(command)
}

fn resolve_owner(explicit: Option<String>) -> Result<String> {
    let configured = if explicit.is_none() {
        crate::config::Config::load()?.whitelisted_pubkeys
    } else {
        Vec::new()
    };
    select_owner(explicit, configured)
}

fn select_owner(explicit: Option<String>, configured: Vec<String>) -> Result<String> {
    let candidate = explicit
        .or_else(|| configured.into_iter().next())
        .context(
            "no whitelisted operator pubkey is configured; pass --owner-pubkey or run `mosaico install`",
        )?;
    PublicKey::parse(candidate.trim())
        .map(|public_key| public_key.to_hex())
        .with_context(|| format!("invalid relay owner public key {candidate:?}"))
}

fn install_embedded() -> Result<PathBuf> {
    let bin_dir = crate::config::mosaico_home().join("relay/bin");
    crate::config::ensure_dir(&bin_dir)?;
    let destination = bin_dir.join(format!("croissant-{}", &CROISSANT_REV[..12]));
    if destination.is_file() {
        ensure_executable(&destination)?;
        return Ok(destination);
    }

    let temporary = bin_dir.join(format!(
        ".croissant-{}-{}",
        &CROISSANT_REV[..12],
        std::process::id()
    ));
    let result = extract_archive(CROISSANT_ARCHIVE, &temporary).and_then(|()| {
        fs::rename(&temporary, &destination).context("installing bundled Croissant")
    });
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    Ok(destination)
}

fn extract_archive(archive: &[u8], destination: &Path) -> Result<()> {
    let source = BufReader::new(Cursor::new(archive));
    let file =
        File::create(destination).with_context(|| format!("creating {}", destination.display()))?;
    let mut output = BufWriter::new(file);
    zstd::stream::copy_decode(source, &mut output).context("extracting bundled Croissant")?;
    output.flush().context("flushing bundled Croissant")?;
    ensure_executable(destination)
}

#[cfg(unix)]
fn ensure_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .with_context(|| format!("making {} executable", path.display()))
}

#[cfg(not(unix))]
fn ensure_executable(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn replace_process(mut command: Command) -> Result<()> {
    use std::os::unix::process::CommandExt as _;
    let error = command.exec();
    Err(error).context("launching bundled Croissant")
}

#[cfg(not(unix))]
fn replace_process(mut command: Command) -> Result<()> {
    let status = command.status().context("launching bundled Croissant")?;
    if !status.success() {
        anyhow::bail!("bundled Croissant exited with {status}");
    }
    Ok(())
}

#[cfg(test)]
#[path = "relay/tests.rs"]
mod tests;
