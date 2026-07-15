//! Compose a named Codex config profile into an isolated app-server home.
//!
//! Codex currently rejects `--profile` for `app-server`, even before the
//! subcommand. We therefore reproduce Codex's user-layer semantics in scratch
//! state: base `config.toml`, deep-merged named profile, then normal `-c`
//! harness overrides remain on argv and retain highest precedence.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::profile::{CodexHomePlan, ProfilePlan};

pub(super) fn source_home() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("CODEX_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Codex app-server profile requires CODEX_HOME or HOME"))?;
    Ok(PathBuf::from(home).join(".codex"))
}

pub(super) fn plan(name: &str, source_home: &Path, scratch: &Path) -> Result<ProfilePlan> {
    validate_name(name)?;
    let base = read_optional_toml(&source_home.join("config.toml"))?;
    let named_path = source_home.join(format!("{name}.config.toml"));
    let named = read_required_toml(&named_path)?;
    let merged = deep_merge(base, named);
    let contents =
        toml::to_string_pretty(&merged).context("serializing composed Codex config profile")?;
    let target_home = scratch.join("codex-home");
    Ok(ProfilePlan {
        extra_env: vec![(
            "CODEX_HOME".to_string(),
            target_home.to_string_lossy().into_owned(),
        )],
        files: vec![(target_home.join("config.toml"), contents)],
        codex_home: Some(CodexHomePlan {
            source: source_home.to_path_buf(),
            target: target_home,
        }),
        ..Default::default()
    })
}

pub(super) fn prepare_home(plan: &CodexHomePlan) -> Result<()> {
    std::fs::create_dir_all(&plan.target)
        .with_context(|| format!("creating staged Codex home {}", plan.target.display()))?;
    let staged_config = plan.target.join("config.toml");
    remove_file_or_symlink(&staged_config)?;

    for entry in std::fs::read_dir(&plan.source)
        .with_context(|| format!("reading source Codex home {}", plan.source.display()))?
    {
        let entry = entry?;
        if entry.file_name() == "config.toml" {
            continue;
        }
        let target = plan.target.join(entry.file_name());
        if std::fs::symlink_metadata(&target).is_ok() {
            continue;
        }
        std::os::unix::fs::symlink(entry.path(), &target).with_context(|| {
            format!(
                "linking Codex home entry {} -> {}",
                target.display(),
                entry.path().display()
            )
        })?;
    }
    Ok(())
}

fn remove_file_or_symlink(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() || meta.is_file() => std::fs::remove_file(path)
            .with_context(|| format!("removing stale staged config {}", path.display())),
        Ok(_) => anyhow::bail!("staged config path is not a file: {}", path.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("inspecting {}", path.display())),
    }
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        anyhow::bail!("invalid Codex profile {name:?}; use only letters, numbers, '-' and '_'");
    }
    Ok(())
}

fn read_optional_toml(path: &Path) -> Result<toml::Value> {
    match std::fs::read_to_string(path) {
        Ok(contents) => parse_toml(path, &contents),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(toml::Value::Table(Default::default()))
        }
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn read_required_toml(path: &Path) -> Result<toml::Value> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("reading named Codex profile {}", path.display()))?;
    parse_toml(path, &contents)
}

fn parse_toml(path: &Path, contents: &str) -> Result<toml::Value> {
    let value: toml::Value = toml::from_str(contents)
        .with_context(|| format!("parsing Codex config {}", path.display()))?;
    if !value.is_table() {
        anyhow::bail!("Codex config root must be a TOML table: {}", path.display());
    }
    Ok(value)
}

fn deep_merge(base: toml::Value, overlay: toml::Value) -> toml::Value {
    match (base, overlay) {
        (toml::Value::Table(mut base), toml::Value::Table(overlay)) => {
            for (key, value) in overlay {
                let merged = match base.remove(&key) {
                    Some(existing) => deep_merge(existing, value),
                    None => value,
                };
                base.insert(key, merged);
            }
            toml::Value::Table(base)
        }
        (_, overlay) => overlay,
    }
}

#[cfg(test)]
#[path = "codex_profile/tests.rs"]
mod tests;
