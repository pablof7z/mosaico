//! JSON-preserving mutation of Mosaico's selected device configuration.

use anyhow::{Context, Result};
use serde_json::Value;
use std::path::Path;

pub(super) fn update(path: &Path, mutate: impl FnOnce(&mut Value) -> Result<()>) -> Result<bool> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut root: Value =
        serde_json::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
    let before = root.clone();
    root.as_object()
        .context("config.json must contain a JSON object")?;
    mutate(&mut root)?;
    if root == before {
        return Ok(false);
    }
    write_pretty(path, &root)?;
    Ok(true)
}

pub(super) fn write_pretty(path: &Path, root: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        super::ensure_dir(parent)?;
    }
    let pretty = serde_json::to_string_pretty(root).context("serializing config json")?;
    std::fs::write(path, pretty).with_context(|| format!("writing {}", path.display()))
}
