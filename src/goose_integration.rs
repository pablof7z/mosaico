//! Goose's awaited lifecycle hooks refresh a per-process Top Of Mind file.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

#[path = "goose_integration/config.rs"]
mod config;
#[cfg(test)]
use config::parse_version;
pub(crate) use config::{
    enable_plugin, is_installed, is_present, plugin_files, plugin_root, validate_runtime,
};

pub(crate) const PLUGIN_JSON: &str = include_str!("../integrations/goose/plugin.json");
pub(crate) const HOOKS_JSON: &str = include_str!("../integrations/goose/hooks/hooks.json");
pub(crate) const MOIM_ENV: &str = "GOOSE_MOIM_MESSAGE_FILE";
const MIN_CONTEXT_LIMIT: usize = 32_000;
const MAX_MOIM_BYTES: usize = 65_536;

fn validate_context_limit() -> Result<()> {
    let Some(raw) = std::env::var_os("GOOSE_CONTEXT_LIMIT") else {
        return Ok(());
    };
    let value = raw.to_string_lossy();
    let limit = value
        .parse::<usize>()
        .with_context(|| format!("invalid GOOSE_CONTEXT_LIMIT {value:?}"))?;
    if limit < MIN_CONTEXT_LIMIT {
        bail!(
            "Goose fabric context requires a declared context window of at least {MIN_CONTEXT_LIMIT} tokens; GOOSE_CONTEXT_LIMIT is {limit}"
        );
    }
    Ok(())
}

pub(crate) fn prepare_launch_env(env: &mut Vec<(String, String)>, endpoint_id: &str) -> Result<()> {
    validate_runtime()?;
    if !is_installed() {
        bail!(
            "Goose fabric integration is missing, stale, or disabled; run `mosaico setup --harness goose` before launching Goose"
        );
    }
    validate_context_limit()?;
    let root = context_root()?;
    std::fs::create_dir_all(&root)
        .with_context(|| format!("creating Goose context directory {}", root.display()))?;
    let path = root.join(format!("{endpoint_id}.md"));
    env.retain(|(key, _)| key != MOIM_ENV);
    env.push((MOIM_ENV.to_string(), path.to_string_lossy().to_string()));
    Ok(())
}

fn context_root() -> Result<PathBuf> {
    Ok(crate::config::mosaico_home().join("harness-context/goose"))
}

pub(crate) fn sync_hook_context(hook_type: &str, context: Option<&str>) -> Result<()> {
    let path = std::env::var_os(MOIM_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .context("Goose hook has no session-specific GOOSE_MOIM_MESSAGE_FILE")?;
    validate_context_path(&path)?;
    match hook_type {
        "user-prompt-submit" | "post-tool-use" => {
            if let Some(delta) = context.filter(|value| !value.trim().is_empty()) {
                append_delta(&path, delta)
            } else {
                Ok(())
            }
        }
        _ => Ok(()),
    }
}

fn validate_context_path(path: &Path) -> Result<()> {
    let root = context_root()?;
    std::fs::create_dir_all(&root)?;
    let canonical_root = root.canonicalize()?;
    let parent = path.parent().context("Goose context file has no parent")?;
    let canonical_parent = parent.canonicalize().with_context(|| {
        format!(
            "Goose context directory {} does not exist",
            parent.display()
        )
    })?;
    if canonical_parent != canonical_root || path.file_name().is_none() {
        bail!(
            "refusing to write Goose context outside {}",
            canonical_root.display()
        );
    }
    Ok(())
}

fn append_delta(path: &Path, delta: &str) -> Result<()> {
    let previous = std::fs::read_to_string(path).unwrap_or_default();
    let combined = if previous.trim().is_empty() {
        delta.to_string()
    } else {
        format!("{delta}\n\n{previous}")
    };
    atomic_write(path, bounded_utf8(&combined))
}

fn bounded_utf8(value: &str) -> &str {
    if value.len() <= MAX_MOIM_BYTES {
        return value;
    }
    let mut end = MAX_MOIM_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn atomic_write(path: &Path, content: &str) -> Result<()> {
    validate_context_path(path)?;
    atomic_write_unchecked(path, content)
}

pub(super) fn atomic_write_unchecked(path: &Path, content: &str) -> Result<()> {
    use std::io::Write as _;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt as _;

    let parent = path.parent().context("file has no parent directory")?;
    std::fs::create_dir_all(parent)?;
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temp)
        .with_context(|| format!("opening temporary Goose context {}", temp.display()))?;
    file.write_all(content.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&temp, path)
        .with_context(|| format!("publishing Goose context {}", path.display()))
}

#[cfg(test)]
#[path = "goose_integration/tests.rs"]
mod tests;
