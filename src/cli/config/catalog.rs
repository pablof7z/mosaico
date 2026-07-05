//! Live model listings fetched straight from the configured provider —
//! Ollama's local `/api/tags` and OpenRouter's public `/api/v1/models` — so
//! the Models menu never shows a stale or hand-maintained list.

use anyhow::{bail, Context, Result};
use serde::Deserialize;

/// One fuzzy-searchable entry in a model picker. `id` is what gets written
/// to `llms.json`; `detail` is a dimmed secondary column (size, context
/// window, price, ...) shown alongside it and is also searchable.
#[derive(Clone)]
pub(super) struct CatalogModel {
    pub(super) id: String,
    pub(super) detail: String,
}

impl std::fmt::Display for CatalogModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.detail.is_empty() {
            write!(f, "{}", self.id)
        } else {
            write!(f, "{}  ({})", self.id, self.detail)
        }
    }
}

/// GET `{base_url}/api/tags` — the models already pulled onto this Ollama
/// instance (localhost, a LAN box, or Ollama Cloud).
pub(super) async fn ollama_models(base_url: &str) -> Result<Vec<CatalogModel>> {
    #[derive(Deserialize)]
    struct Tags {
        #[serde(default)]
        models: Vec<Tag>,
    }
    #[derive(Deserialize)]
    struct Tag {
        name: String,
        #[serde(default)]
        size: Option<u64>,
    }

    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let resp = reqwest::get(&url)
        .await
        .with_context(|| format!("connecting to Ollama at {base_url}"))?;
    if !resp.status().is_success() {
        bail!("Ollama at {base_url} returned HTTP {}", resp.status());
    }
    let tags: Tags = resp
        .json()
        .await
        .context("parsing Ollama /api/tags response")?;

    Ok(tags
        .models
        .into_iter()
        .map(|t| CatalogModel {
            detail: t.size.map(format_gb).unwrap_or_default(),
            id: t.name,
        })
        .collect())
}

fn format_gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
}

/// GET `https://openrouter.ai/api/v1/models` — the full OpenRouter catalog
/// (hundreds of entries), annotated with context window and prompt price.
pub(super) async fn openrouter_models(api_key: &str) -> Result<Vec<CatalogModel>> {
    #[derive(Deserialize)]
    struct ModelsResponse {
        data: Vec<Model>,
    }
    #[derive(Deserialize)]
    struct Model {
        id: String,
        #[serde(default)]
        context_length: Option<u64>,
        #[serde(default)]
        pricing: Option<Pricing>,
    }
    #[derive(Deserialize)]
    struct Pricing {
        #[serde(default)]
        prompt: Option<String>,
    }

    let client = reqwest::Client::new();
    let mut req = client.get("https://openrouter.ai/api/v1/models");
    if !api_key.is_empty() {
        req = req.bearer_auth(api_key);
    }
    let resp = req.send().await.context("connecting to OpenRouter")?;
    if !resp.status().is_success() {
        bail!("OpenRouter returned HTTP {}", resp.status());
    }
    let parsed: ModelsResponse = resp
        .json()
        .await
        .context("parsing OpenRouter /models response")?;

    Ok(parsed
        .data
        .into_iter()
        .map(|m| {
            let mut parts = Vec::new();
            if let Some(ctx) = m.context_length {
                parts.push(format!("{}k ctx", ctx / 1000));
            }
            if let Some(price) = m
                .pricing
                .and_then(|p| p.prompt)
                .and_then(|p| p.parse::<f64>().ok())
            {
                parts.push(if price > 0.0 {
                    format!("${:.2}/M in", price * 1_000_000.0)
                } else {
                    "free".to_string()
                });
            }
            CatalogModel {
                id: m.id,
                detail: parts.join(" · "),
            }
        })
        .collect())
}
