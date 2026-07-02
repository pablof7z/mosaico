use super::*;

pub(in crate::daemon::server) async fn rpc_publish_profile(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        slug: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("publish_profile params")?;

    let edge_home = crate::config::edge_home();
    let id = crate::identity::load_or_create(&edge_home, &p.slug, now_secs())
        .with_context(|| format!("loading agent {}", p.slug))?;

    let ev = DomainEvent::Profile(crate::domain::Profile {
        agent: crate::domain::AgentRef::new(id.pubkey_hex(), p.slug.clone()),
        host: state.host.clone(),
        owners: state.owners.clone(),
        is_backend: false,
    });
    let event_id = state.provider.publish(&ev, &id.keys).await?;

    Ok(serde_json::json!({
        "slug": p.slug,
        "pubkey": id.pubkey_hex(),
        "event_id": event_id.to_hex(),
    }))
}

/// Resolve a backend label (from `slug@backend-label`) to the backend's pubkey.
/// The label is exactly config.json `backendName`; it is not an OS/DNS hostname,
/// pubkey, npub, NIP-05 address, or slugified display string.
pub(in crate::daemon::server) async fn resolve_backend_pubkey(
    state: &Arc<DaemonState>,
    label: &str,
) -> Result<String> {
    if label == state.host {
        return state.backend_pubkey.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "backend label {label:?} matches local backend but no signing key is configured"
            )
        });
    }

    if let Some(pk) = state.with_store(|s| s.pubkey_for_backend_label(label).ok().flatten()) {
        return Ok(pk);
    }

    anyhow::bail!(
        "cannot resolve backend label {label:?}: no backend profile advertises that config.json backendName"
    )
}

pub(in crate::daemon::server) async fn resolve_project_member_pubkey_hex(
    input: &str,
) -> Result<String> {
    let edge_home = config::edge_home();
    if let Some(agent) = identity::list_local_agent_details(&edge_home)
        .into_iter()
        .find(|agent| agent.slug == input)
    {
        return Ok(agent.pubkey);
    }

    resolve_pubkey_hex(input).await.with_context(|| {
        format!("resolving {input:?} as a local agent slug, pubkey, npub, or NIP-05 address")
    })
}

pub(in crate::daemon::server) async fn resolve_pubkey_hex(input: &str) -> Result<String> {
    use nostr_sdk::prelude::PublicKey;

    // hex / npub / nostr: URI
    if let Ok(pk) = PublicKey::parse(input) {
        return Ok(pk.to_hex());
    }

    // NIP-05: name@domain
    if let Some((name, domain)) = input.split_once('@') {
        if !domain.is_empty() {
            let url = format!("https://{domain}/.well-known/nostr.json?name={name}");
            let json: serde_json::Value = reqwest::get(url)
                .await
                .with_context(|| format!("NIP-05 HTTP request to {domain} failed"))?
                .json()
                .await
                .context("NIP-05 response is not valid JSON")?;
            let hex = json["names"][name]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("NIP-05: name {name:?} not found at {domain}"))?;
            return PublicKey::from_hex(hex)
                .map(|pk| pk.to_hex())
                .context("NIP-05 returned invalid pubkey");
        }
    }

    anyhow::bail!("cannot parse {input:?} as pubkey (hex/npub) or NIP-05 (user@domain)")
}

// ── chat read (backfill + optional live stream) ───────────────────────────────
