//! Persistence for the daemon-owned management key (`mosaicoPrivateKey`).

use anyhow::{Context, Result};
use nostr::Keys;
use serde_json::Value;
use std::path::Path;

pub(crate) fn generate_mosaico_private_key() -> String {
    Keys::generate().secret_key().to_secret_hex()
}

pub(crate) fn ensure_mosaico_private_key() -> Result<String> {
    ensure_mosaico_private_key_at(&super::config_path(), generate_mosaico_private_key)
}

fn ensure_mosaico_private_key_at(path: &Path, generate: impl FnOnce() -> String) -> Result<String> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut root: Value =
        serde_json::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
    let (key, changed) = ensure_mosaico_private_key_value(&mut root, generate)?;
    if changed {
        write_pretty(path, &root)?;
        tracing::info!(
            config = %path.display(),
            "generated missing mosaicoPrivateKey for daemon management"
        );
    }
    Ok(key)
}

fn ensure_mosaico_private_key_value(
    root: &mut Value,
    generate: impl FnOnce() -> String,
) -> Result<(String, bool)> {
    let object = root
        .as_object_mut()
        .context("config.json must be a JSON object to add mosaicoPrivateKey")?;
    if let Some(existing) = object
        .get("mosaicoPrivateKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Ok((existing.to_string(), false));
    }

    let generated = generate();
    object.insert(
        "mosaicoPrivateKey".to_string(),
        Value::String(generated.clone()),
    );
    Ok((generated, true))
}

fn write_pretty(path: &Path, root: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        super::ensure_dir(parent)?;
    }
    let pretty = serde_json::to_string_pretty(root).context("serializing config json")?;
    std::fs::write(path, pretty).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_missing_key_and_preserves_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"version":3,"backendName":"test","relays":["wss://relay.example"]}"#,
        )
        .unwrap();

        let key = ensure_mosaico_private_key_at(&path, || "backend-secret".to_string()).unwrap();

        assert_eq!(key, "backend-secret");
        let root: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(root["version"], 3);
        assert_eq!(root["mosaicoPrivateKey"], "backend-secret");
    }

    #[test]
    fn preserves_existing_key() {
        let mut root = serde_json::json!({
            "backendName": "test",
            "mosaicoPrivateKey": "existing-secret",
        });

        let (key, changed) =
            ensure_mosaico_private_key_value(&mut root, || "new-secret".to_string()).unwrap();

        assert_eq!(key, "existing-secret");
        assert!(!changed);
        assert_eq!(root["mosaicoPrivateKey"], "existing-secret");
    }

    #[test]
    fn replaces_blank_key() {
        let mut root = serde_json::json!({
            "backendName": "test",
            "mosaicoPrivateKey": "   ",
        });

        let (key, changed) =
            ensure_mosaico_private_key_value(&mut root, || "backend-secret".to_string()).unwrap();

        assert_eq!(key, "backend-secret");
        assert!(changed);
        assert_eq!(root["mosaicoPrivateKey"], "backend-secret");
    }
}
