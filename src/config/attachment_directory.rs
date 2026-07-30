use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Component;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct AttachmentConfig {
    #[serde(default, rename = "attachmentReceiveDirectory")]
    directory: Option<PathBuf>,
}

pub(super) fn resolve(value: Option<PathBuf>, home: &Path) -> Result<PathBuf> {
    let home = lexical_absolute(home)?;
    let selected = match value.filter(|path| !path.as_os_str().is_empty()) {
        Some(path) if path.is_absolute() => path,
        Some(path) => home.join(path),
        None => home.join("tmp/attachments"),
    };
    lexical_absolute(&selected)
}

fn lexical_absolute(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("resolving current directory for attachment storage")?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(std::path::MAIN_SEPARATOR_STR),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    Ok(normalized)
}

pub(super) fn ensure() -> Result<PathBuf> {
    let path = super::config_path();
    let home = super::mosaico_home();
    let content =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let parsed: AttachmentConfig =
        serde_json::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
    let resolved = resolve(parsed.directory, &home)?;
    super::document::update(&path, |root| {
        let object = root
            .as_object_mut()
            .context("config.json must contain a JSON object")?;
        object.insert(
            "attachmentReceiveDirectory".to_string(),
            serde_json::Value::String(resolved.display().to_string()),
        );
        Ok(())
    })?;
    Ok(resolved)
}
