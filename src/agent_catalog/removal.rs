use super::NativeAgentProfile;
use crate::session::Harness;
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

/// Permanently delete one exact, already-discovered native profile source.
///
/// Callers resolve the profile through the catalog first, which disambiguates
/// same-named profiles by harness and scope without accepting an arbitrary path.
pub fn remove_native_profile(profile: &NativeAgentProfile) -> Result<bool> {
    if profile.harness == Harness::Hermes {
        return remove_hermes_profile(profile);
    }
    match std::fs::remove_file(&profile.path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error)
            .with_context(|| format!("deleting native agent profile {}", profile.path.display())),
    }
}

fn remove_hermes_profile(profile: &NativeAgentProfile) -> Result<bool> {
    if !profile.path.exists() {
        return Ok(false);
    }
    let root = hermes_root(profile)?;
    let output = Command::new("hermes")
        .env("HERMES_HOME", root)
        .args(["profile", "delete", &profile.slug, "--yes"])
        .output()
        .with_context(|| format!("deleting Hermes profile {:?}", profile.slug))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "hermes profile delete {:?} failed{}",
            profile.slug,
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        );
    }
    Ok(true)
}

fn hermes_root(profile: &NativeAgentProfile) -> Result<&Path> {
    let name = profile.path.file_name().and_then(|value| value.to_str());
    let profiles = profile
        .path
        .parent()
        .filter(|path| path.ends_with("profiles"));
    if name != Some(profile.slug.as_str()) || profiles.is_none() {
        anyhow::bail!(
            "refusing to delete Hermes profile outside its exact profiles directory: {}",
            profile.path.display()
        );
    }
    profiles
        .and_then(Path::parent)
        .context("Hermes profiles directory has no root")
}
