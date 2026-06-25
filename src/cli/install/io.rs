//! File I/O for JSON and text configuration files.

use anyhow::{Context, Result};
use std::path::Path;

pub fn read_json_or_default(path: &Path) -> Result<serde_json::Value> {
    let mut root = match std::fs::read_to_string(path) {
        Ok(content) if content.trim().is_empty() => serde_json::json!({}),
        Ok(content) => serde_json::from_str(&content)
            .with_context(|| format!("{} is not valid JSON", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    super::hooks::ensure_hooks_object(&mut root);
    Ok(root)
}

pub fn print_json_preview(v: &serde_json::Value) -> Result<()> {
    let pretty = serde_json::to_string_pretty(v)?;
    for line in pretty.lines() {
        println!("    {line}");
    }
    Ok(())
}

pub fn write_json(path: &Path, v: &serde_json::Value) -> Result<()> {
    let pretty = serde_json::to_string_pretty(v)?;
    write_text(path, &(pretty + "\n"))
}

pub fn write_text(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text)?;
    Ok(())
}
