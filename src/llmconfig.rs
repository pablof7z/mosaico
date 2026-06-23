//! Resolve a TENEX **role** (e.g. `edge-distillation`) to a concrete model +
//! credentials, reading the EXISTING TENEX config format from `~/.tenex-edge`:
//!
//!   - `providers.json`: `{ "providers": { "<provider>": { "apiKey": ... } } }`
//!     where for `ollama` the `apiKey` field actually holds the **base URL**.
//!   - `llms.json`: a `"configurations"` map of named entries
//!     `{ "model": "...", "provider": "..." }`, PLUS top-level **role keys**
//!     mapping a role name → a configuration name.
//!
//! Resolution: role → `llms.json[role]` (a config name) →
//! `configurations[name]` → `{provider, model}` → creds from
//! `providers.json["providers"][provider]["apiKey"]`.
//!
//! Only `openrouter` and `ollama` are supported for distillation; any other
//! provider (acp/meta/anthropic/codex/…) resolves to `None` so the caller falls
//! back to the heuristic distiller.

use serde_json::Value;
use std::path::Path;

/// A fully resolved model ready to hand to the rig distiller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModel {
    pub provider: String,
    pub model: String,
    /// API key (openrouter). Empty for ollama.
    pub api_key: String,
    /// Base URL (ollama). Empty for openrouter (rig uses its own default).
    pub base_url: String,
}

/// Resolve a role using the default `tenex_dir()`.
pub fn resolve_role(role: &str) -> Option<ResolvedModel> {
    resolve_role_in(&crate::config::tenex_dir(), role)
}

/// Pure core: resolve `role` against the `providers.json` + `llms.json` found in
/// `dir`. Returns `None` if anything is missing or the provider is unsupported.
/// Kept env-free so tests can drive it with a `tempfile` dir (no global env races).
pub fn resolve_role_in(dir: &Path, role: &str) -> Option<ResolvedModel> {
    let providers: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("providers.json")).ok()?).ok()?;
    let llms: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("llms.json")).ok()?).ok()?;

    // role -> configuration name
    let config_name = llms.get(role)?.as_str()?;
    // configuration name -> { provider, model }
    let conf = llms.get("configurations")?.get(config_name)?;
    let provider = conf.get("provider")?.as_str()?.to_string();
    let model = conf.get("model")?.as_str()?.to_string();

    // provider -> apiKey (string or array; take first if array)
    let api_key_field = providers.get("providers")?.get(&provider)?.get("apiKey")?;
    let api_key = match api_key_field {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.first().and_then(|v| v.as_str()).unwrap_or("").to_string(),
        _ => return None,
    };

    match provider.as_str() {
        "openrouter" => Some(ResolvedModel {
            provider,
            model,
            api_key,
            base_url: String::new(),
        }),
        // For ollama the provider's `apiKey` field is the base URL.
        "ollama" => Some(ResolvedModel {
            provider,
            model,
            api_key: String::new(),
            base_url: api_key,
        }),
        // acp / meta / anthropic / codex / … are not supported here → heuristic.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_dir(providers: &str, llms: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("providers.json"), providers).unwrap();
        fs::write(d.path().join("llms.json"), llms).unwrap();
        d
    }

    const PROVIDERS: &str = r#"{
        "providers": {
            "openrouter": { "apiKey": "sk-or-test" },
            "ollama": { "apiKey": "http://localhost:11434" },
            "anthropic": { "apiKey": ["sk-ant-first", "sk-ant-second"] }
        }
    }"#;

    const LLMS: &str = r#"{
        "configurations": {
            "openrouter/openai/gpt-4o-mini": { "model": "openai/gpt-4o-mini", "provider": "openrouter" },
            "ollama/deepseek-v4-flash:cloud": { "model": "deepseek-v4-flash:cloud", "provider": "ollama" },
            "anthropic-conf": { "model": "claude", "provider": "anthropic" }
        },
        "edge-distillation": "openrouter/openai/gpt-4o-mini"
    }"#;

    #[test]
    fn resolves_openrouter_role() {
        let d = write_dir(PROVIDERS, LLMS);
        let r = resolve_role_in(d.path(), "edge-distillation").unwrap();
        assert_eq!(r.provider, "openrouter");
        assert_eq!(r.model, "openai/gpt-4o-mini");
        assert_eq!(r.api_key, "sk-or-test");
        assert_eq!(r.base_url, "");
    }

    #[test]
    fn resolves_ollama_base_url_from_apikey_field() {
        let llms = r#"{
            "configurations": {
                "ollama/deepseek-v4-flash:cloud": { "model": "deepseek-v4-flash:cloud", "provider": "ollama" }
            },
            "edge-distillation": "ollama/deepseek-v4-flash:cloud"
        }"#;
        let d = write_dir(PROVIDERS, llms);
        let r = resolve_role_in(d.path(), "edge-distillation").unwrap();
        assert_eq!(r.provider, "ollama");
        assert_eq!(r.model, "deepseek-v4-flash:cloud");
        assert_eq!(r.base_url, "http://localhost:11434");
        assert_eq!(r.api_key, "");
    }

    #[test]
    fn unsupported_provider_resolves_none() {
        let llms = r#"{
            "configurations": { "anthropic-conf": { "model": "claude", "provider": "anthropic" } },
            "edge-distillation": "anthropic-conf"
        }"#;
        let d = write_dir(PROVIDERS, llms);
        assert!(resolve_role_in(d.path(), "edge-distillation").is_none());
    }

    #[test]
    fn missing_role_resolves_none() {
        let d = write_dir(PROVIDERS, LLMS);
        assert!(resolve_role_in(d.path(), "no-such-role").is_none());
    }

    #[test]
    fn array_apikey_takes_first() {
        // Sanity: array apiKey handling (even though anthropic itself is unsupported,
        // an array-keyed openrouter would behave the same).
        let providers =
            r#"{ "providers": { "openrouter": { "apiKey": ["sk-or-A", "sk-or-B"] } } }"#;
        let d = write_dir(providers, LLMS);
        let r = resolve_role_in(d.path(), "edge-distillation").unwrap();
        assert_eq!(r.api_key, "sk-or-A");
    }
}
