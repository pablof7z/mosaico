//! What a dynamic client registration *is* here: a durable record that a
//! `client_id` may receive codes at exactly these redirect URIs.
//!
//! Before mosaico#766 `/oauth/register` echoed the posted `redirect_uris` back
//! with a freshly hashed `client_id` and stored nothing, so `/oauth/authorize`
//! had nothing to check a `redirect_uri` against and accepted any absolute URL.
//! A registration that persists nothing cannot be the basis of a later check.
//!
//! Two properties follow from making it durable:
//!
//! - The registry outlives the MCP server process. Tokens are stateless and
//!   already survive a restart; a client that re-authorizes after one must not
//!   discover its `client_id` has evaporated.
//! - The `client_id` is derived from the registered redirect set, so
//!   re-registering the same client is idempotent instead of growing the file.
//!   It is an opaque identifier, not a secret: it confers nothing beyond the
//!   redirect set it names, and that set was screened by
//!   [`super::auth_redirect`] before it was written.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::auth_support::stable_hash;

/// Registrations retained. Anything that reaches `/oauth/register` can add one,
/// so the file is bounded and the least recently registered entry is dropped
/// first. An evicted client re-registers — that is what DCR is for.
const MAX_REGISTERED_CLIENTS: usize = 64;

#[derive(Clone)]
pub(super) struct ClientRegistry {
    path: PathBuf,
    clients: Arc<Mutex<HashMap<String, RegisteredClient>>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(super) struct RegisteredClient {
    redirect_uris: Vec<String>,
    registered_at: u64,
}

#[derive(Default, Deserialize, Serialize)]
struct Document {
    #[serde(default)]
    clients: HashMap<String, RegisteredClient>,
}

impl ClientRegistry {
    /// Open the registry at `path`.
    ///
    /// A registry that cannot be read or parsed opens empty, which denies every
    /// authorize request until a client registers again. That is the safe
    /// direction: the alternative to "no registrations" is not "all
    /// registrations", it is a door that admits redirect targets nobody
    /// recorded.
    pub(super) fn open(path: PathBuf) -> Self {
        let clients = match read_document(&path) {
            Ok(document) => document.clients,
            Err(err) => {
                if path.exists() {
                    tracing::warn!(
                        registry = %path.display(),
                        error = %format!("{err:#}"),
                        "MCP client registry is unreadable; opening empty so no unrecorded \
                         redirect target can be authorized"
                    );
                }
                HashMap::new()
            }
        };
        Self {
            path,
            clients: Arc::new(Mutex::new(clients)),
        }
    }

    /// Record a registration and return the metadata document for the response.
    pub(super) fn register(&self, body: &Value, approved_origins: &[String]) -> Result<Value> {
        let requested = body
            .get("redirect_uris")
            .and_then(Value::as_array)
            .filter(|uris| !uris.is_empty())
            .context("redirect_uris must be a non-empty array")?;
        let mut redirect_uris = Vec::with_capacity(requested.len());
        for uri in requested {
            let uri = uri
                .as_str()
                .context("every redirect_uris entry must be a string")?;
            redirect_uris.push(super::auth_redirect::accept(uri, approved_origins)?);
        }
        redirect_uris.sort();
        redirect_uris.dedup();

        let client_id = client_id_for(&redirect_uris);
        let record = RegisteredClient {
            redirect_uris: redirect_uris.clone(),
            registered_at: crate::util::now_secs(),
        };
        {
            let mut clients = self.lock();
            clients.insert(client_id.clone(), record);
            evict_oldest_beyond_cap(&mut clients);
            write_document(&self.path, &clients)?;
        }
        Ok(json!({
            "client_id": client_id,
            "redirect_uris": redirect_uris,
            "grant_types": ["authorization_code"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        }))
    }

    /// Refuse anything but an exact match against what this `client_id`
    /// registered. Prefix and origin matching are both wrong here: a client
    /// registered at `https://client.example/callback` must not receive codes
    /// at `https://client.example/callback.evil` or `.../callback/../elsewhere`.
    pub(super) fn ensure_registered(&self, client_id: &str, redirect_uri: &str) -> Result<()> {
        let clients = self.lock();
        let Some(client) = clients.get(client_id) else {
            anyhow::bail!("unknown client_id: register at /oauth/register first");
        };
        anyhow::ensure!(
            client
                .redirect_uris
                .iter()
                .any(|registered| registered == redirect_uri),
            "redirect_uri is not registered for this client_id"
        );
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, RegisteredClient>> {
        self.clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn client_id_for(redirect_uris: &[String]) -> String {
    format!("mcpc_{}", stable_hash(&json!(redirect_uris)))
}

fn evict_oldest_beyond_cap(clients: &mut HashMap<String, RegisteredClient>) {
    while clients.len() > MAX_REGISTERED_CLIENTS {
        let Some(oldest) = clients
            .iter()
            .min_by_key(|(id, client)| (client.registered_at, (*id).clone()))
            .map(|(id, _)| id.clone())
        else {
            return;
        };
        clients.remove(&oldest);
    }
}

fn read_document(path: &Path) -> Result<Document> {
    let body =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&body).with_context(|| format!("parsing {}", path.display()))
}

fn write_document(path: &Path, clients: &HashMap<String, RegisteredClient>) -> Result<()> {
    if let Some(parent) = path.parent() {
        crate::config::ensure_dir(parent)?;
    }
    let document = Document {
        clients: clients.clone(),
    };
    let body =
        serde_json::to_string_pretty(&document).context("serializing MCP client registry")?;
    let staging = path.with_extension("json.writing");
    std::fs::write(&staging, body).with_context(|| format!("writing {}", staging.display()))?;
    std::fs::rename(&staging, path)
        .with_context(|| format!("replacing {} with {}", path.display(), staging.display()))
}

#[cfg(test)]
#[path = "auth_clients/tests.rs"]
mod tests;
