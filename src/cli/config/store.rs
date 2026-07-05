//! Read/modify/write `providers.json` and `llms.json` under `edge_home()`.
//!
//! Both files are shared with the wider TENEX config format (see
//! `crate::llmconfig`'s module docs), so mutations preserve every key this
//! tool doesn't understand — the whole document is held as a
//! [`serde_json::Value`] and only the paths we edit are touched.

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// `~/.tenex-edge/providers.json`.
pub(super) struct ProvidersFile {
    path: PathBuf,
    root: Value,
}

impl ProvidersFile {
    pub(super) fn load() -> Result<Self> {
        Self::load_in(&crate::config::edge_home())
    }

    fn load_in(dir: &Path) -> Result<Self> {
        let path = dir.join("providers.json");
        let root = match std::fs::read_to_string(&path) {
            Ok(s) => {
                serde_json::from_str(&s).with_context(|| format!("parsing {}", path.display()))?
            }
            Err(_) => json!({ "providers": {} }),
        };
        Ok(Self { path, root })
    }

    /// `(name, display value)` pairs sorted by name. An array-valued
    /// `apiKey` (key rotation) collapses to its first entry for display,
    /// matching `resolve_role_in`'s read semantics.
    pub(super) fn list(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .root
            .get("providers")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(name, v)| (name.clone(), display_value(v.get("apiKey"))))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub(super) fn get(&self, name: &str) -> Option<String> {
        let value = display_value(self.root.get("providers")?.get(name)?.get("apiKey"));
        if value.is_empty() {
            None
        } else {
            Some(value)
        }
    }

    /// Insert or overwrite a provider's `apiKey` field (an API key for most
    /// providers, a base URL for `ollama`). Always writes a plain string —
    /// existing array-valued (rotated) keys are collapsed on edit.
    pub(super) fn set(&mut self, name: &str, value: &str) {
        let root = self.root.as_object_mut().expect("root is always an object");
        let providers = root
            .entry("providers".to_string())
            .or_insert_with(|| json!({}));
        if let Some(map) = providers.as_object_mut() {
            map.insert(name.to_string(), json!({ "apiKey": value }));
        }
    }

    pub(super) fn remove(&mut self, name: &str) {
        if let Some(map) = self
            .root
            .get_mut("providers")
            .and_then(Value::as_object_mut)
        {
            map.remove(name);
        }
    }

    pub(super) fn save(&self) -> Result<()> {
        write_pretty(&self.path, &self.root)
    }
}

fn display_value(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a.first().and_then(Value::as_str).unwrap_or("").to_string(),
        _ => String::new(),
    }
}

/// `~/.tenex-edge/llms.json`.
pub(super) struct LlmsFile {
    path: PathBuf,
    root: Value,
}

impl LlmsFile {
    pub(super) fn load() -> Result<Self> {
        Self::load_in(&crate::config::edge_home())
    }

    fn load_in(dir: &Path) -> Result<Self> {
        let path = dir.join("llms.json");
        let root = match std::fs::read_to_string(&path) {
            Ok(s) => {
                serde_json::from_str(&s).with_context(|| format!("parsing {}", path.display()))?
            }
            Err(_) => json!({ "configurations": {} }),
        };
        Ok(Self { path, root })
    }

    /// Role name -> configuration name, for every top-level key except
    /// `configurations`, sorted by role name.
    pub(super) fn roles(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .root
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(k, _)| k.as_str() != "configurations")
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// `(provider, model)` for a named configuration.
    pub(super) fn configuration(&self, name: &str) -> Option<(String, String)> {
        let conf = self.root.get("configurations")?.get(name)?;
        Some((
            conf.get("provider")?.as_str()?.to_string(),
            conf.get("model")?.as_str()?.to_string(),
        ))
    }

    /// Insert/replace a `{provider}/{model}`-named configuration and point
    /// `role` at it. Returns the configuration name written.
    pub(super) fn set_role(&mut self, role: &str, provider: &str, model: &str) -> String {
        let config_name = format!("{provider}/{model}");
        let root = self.root.as_object_mut().expect("root is always an object");
        let configurations = root
            .entry("configurations".to_string())
            .or_insert_with(|| json!({}));
        if let Some(map) = configurations.as_object_mut() {
            map.insert(
                config_name.clone(),
                json!({ "provider": provider, "model": model }),
            );
        }
        root.insert(role.to_string(), Value::String(config_name.clone()));
        config_name
    }

    pub(super) fn save(&self) -> Result<()> {
        write_pretty(&self.path, &self.root)
    }
}

fn write_pretty(path: &Path, root: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        crate::config::ensure_dir(parent)?;
    }
    let pretty = serde_json::to_string_pretty(root).context("serializing config json")?;
    std::fs::write(path, pretty).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = ProvidersFile::load_in(dir.path()).unwrap();
        assert!(file.list().is_empty());

        file.set("ollama", "http://localhost:11434");
        file.set("openrouter", "sk-or-test");
        file.save().unwrap();

        let reloaded = ProvidersFile::load_in(dir.path()).unwrap();
        assert_eq!(
            reloaded.get("ollama"),
            Some("http://localhost:11434".to_string())
        );
        assert_eq!(reloaded.list().len(), 2);

        let mut reloaded = reloaded;
        reloaded.remove("ollama");
        assert_eq!(reloaded.get("ollama"), None);
    }

    #[test]
    fn array_api_key_collapses_to_first_for_display() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{ "providers": { "anthropic": { "apiKey": ["sk-a", "sk-b"] } } }"#,
        )
        .unwrap();

        let file = ProvidersFile::load_in(dir.path()).unwrap();
        assert_eq!(file.get("anthropic"), Some("sk-a".to_string()));
    }

    #[test]
    fn llms_set_role_creates_configuration_and_role() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = LlmsFile::load_in(dir.path()).unwrap();
        assert!(file.roles().is_empty());

        let name = file.set_role("edge-distillation", "openrouter", "openai/gpt-4o-mini");
        assert_eq!(name, "openrouter/openai/gpt-4o-mini");
        file.save().unwrap();

        let reloaded = LlmsFile::load_in(dir.path()).unwrap();
        assert_eq!(
            reloaded.roles(),
            vec![(
                "edge-distillation".to_string(),
                "openrouter/openai/gpt-4o-mini".to_string()
            )]
        );
        assert_eq!(
            reloaded.configuration("openrouter/openai/gpt-4o-mini"),
            Some(("openrouter".to_string(), "openai/gpt-4o-mini".to_string()))
        );
    }

    #[test]
    fn llms_preserves_unknown_top_level_fields() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("llms.json"),
            r#"{ "configurations": {}, "someOtherHostRole": "some-config" }"#,
        )
        .unwrap();

        let mut file = LlmsFile::load_in(dir.path()).unwrap();
        file.set_role("edge-distillation", "ollama", "llama3");
        file.save().unwrap();

        let raw = std::fs::read_to_string(dir.path().join("llms.json")).unwrap();
        assert!(raw.contains("someOtherHostRole"));
    }
}
