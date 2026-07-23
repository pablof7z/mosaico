use super::InstallOpts;
use anyhow::{bail, Context, Result};
use nostr::{Keys, PublicKey};
use serde_json::{json, Value};
use std::path::Path;

pub(super) fn baseline_document() -> Value {
    json!({
        "whitelistedPubkeys": [],
        "relays": [],
        "indexerRelay": crate::config::DEFAULT_INDEXER_RELAY,
        "backendName": crate::config::hostname(),
        "mosaicoPrivateKey": crate::config::generate_mosaico_private_key(),
        "perSessionRooms": false,
    })
}

pub(super) fn read_document(path: &Path) -> Result<Value> {
    let body =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let value: Value =
        serde_json::from_str(&body).with_context(|| format!("parsing {}", path.display()))?;
    if !value.is_object() {
        bail!(
            "{} must contain a JSON object; refusing to overwrite it",
            path.display()
        );
    }
    Ok(value)
}

pub(super) fn has_overrides(opts: &InstallOpts) -> bool {
    !opts.relay.is_empty()
        || opts.host_label.is_some()
        || opts.operator_pubkeys.is_some()
        || opts.clear_operators
        || opts.operator_nsec_file.is_some()
        || opts.clear_operator_nsec
        || opts.indexer_relay.is_some()
        || opts.per_session_rooms.is_some()
}

pub(super) fn apply_overrides(doc: &mut Value, opts: &InstallOpts) -> Result<()> {
    let object = doc.as_object_mut().expect("configuration is an object");
    if !opts.relay.is_empty() {
        object.insert("relays".into(), json!(normalize_relays(&opts.relay)?));
    }
    if let Some(label) = &opts.host_label {
        object.insert("backendName".into(), json!(normalize_label(label)?));
    }
    if opts.clear_operators {
        object.insert("whitelistedPubkeys".into(), json!([]));
    } else if let Some(pubkeys) = &opts.operator_pubkeys {
        object.insert(
            "whitelistedPubkeys".into(),
            json!(normalize_pubkeys(pubkeys)?),
        );
    }
    if opts.clear_operator_nsec {
        object.remove("userNsec");
    } else if let Some(path) = &opts.operator_nsec_file {
        let secret = std::fs::read_to_string(path)
            .with_context(|| format!("reading operator signing key from {}", path.display()))?;
        object.insert("userNsec".into(), json!(normalize_secret(&secret)?));
    }
    if let Some(relay) = &opts.indexer_relay {
        object.insert("indexerRelay".into(), json!(normalize_relay(relay)?));
    }
    if let Some(enabled) = opts.per_session_rooms {
        object.insert("perSessionRooms".into(), json!(enabled));
    }
    Ok(())
}

pub(super) fn ensure_complete(doc: &mut Value) -> Result<()> {
    let config =
        crate::config::Config::from_json_str(&doc.to_string(), &crate::config::hostname())?;
    let relays = normalize_relays(&config.relays)?;
    let indexer = normalize_relay(&config.indexer_relay)?;
    let host = normalize_label(&config.host)?;
    let operators = normalize_pubkey_list(&config.whitelisted_pubkeys)?;
    let object = doc.as_object_mut().expect("configuration is an object");
    object.insert("relays".into(), json!(relays));
    object.insert("indexerRelay".into(), json!(indexer));
    object.insert("backendName".into(), json!(host));
    object.insert("whitelistedPubkeys".into(), json!(operators));
    object.insert("perSessionRooms".into(), json!(config.per_session_rooms));
    if let Some(secret) = config.backend_nsec() {
        Keys::parse(secret.trim()).context(
            "config contains an invalid mosaicoPrivateKey; refusing to rotate backend identity",
        )?;
    } else {
        object.insert(
            "mosaicoPrivateKey".into(),
            json!(crate::config::generate_mosaico_private_key()),
        );
    }
    if let Some(secret) = config.user_nsec() {
        normalize_secret(secret).context("config contains an invalid userNsec")?;
    }
    Ok(())
}

pub(super) fn print_summary(doc: &Value) {
    let config = crate::config::Config::from_json_str(&doc.to_string(), &crate::config::hostname())
        .expect("validated configuration");
    println!("  host label: {}", config.host);
    println!("  relay(s): {}", config.relays.join(", "));
    println!("  profile indexer: {}", config.indexer_relay);
    println!("  operators: {}", config.whitelisted_pubkeys.len());
    println!(
        "  CLI operator signing key: {}",
        if config.user_nsec().is_some() {
            "present"
        } else {
            "not set"
        }
    );
    println!("  per-session rooms: {}", config.per_session_rooms);
    println!("  backend identity: present (secret not displayed)");
}

pub(super) fn normalize_label(label: &str) -> Result<String> {
    let label = label.trim();
    if label.is_empty() {
        bail!("host label cannot be empty");
    }
    Ok(label.to_string())
}

pub(super) fn normalize_relays(relays: &[String]) -> Result<Vec<String>> {
    if relays.is_empty() {
        bail!("configure at least one externally operated relay URL");
    }
    relays.iter().map(|relay| normalize_relay(relay)).collect()
}

pub(super) fn normalize_relay(relay: &str) -> Result<String> {
    let relay = relay.trim();
    let parsed = url::Url::parse(relay).with_context(|| format!("invalid relay URL {relay:?}"))?;
    if !matches!(parsed.scheme(), "ws" | "wss") {
        bail!("relay URL must use ws:// or wss://: {relay}");
    }
    Ok(parsed.to_string().trim_end_matches('/').to_string())
}

pub(super) fn normalize_pubkeys(raw: &str) -> Result<Vec<String>> {
    normalize_pubkey_list(&split_csv(raw))
}

pub(super) fn normalize_secret(raw: &str) -> Result<String> {
    let secret = raw.trim();
    Keys::parse(secret).context("invalid Nostr secret key")?;
    Ok(secret.to_string())
}

fn normalize_pubkey_list(pubkeys: &[String]) -> Result<Vec<String>> {
    pubkeys
        .iter()
        .map(|value| {
            let value = value.trim();
            PublicKey::parse(value)
                .map(|key| key.to_hex())
                .with_context(|| format!("invalid operator pubkey {value:?}"))
        })
        .collect()
}

pub(super) fn split_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

pub(super) fn missing_management_key(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(true);
    }
    Ok(read_document(path)?.get("mosaicoPrivateKey").is_none())
}
