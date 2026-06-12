use super::demux::event_tag;
use super::lifecycle::resubscribe;
use super::*;

// ── acl ────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct AclParams {
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    target: Option<String>,
}

pub(super) async fn rpc_acl(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: AclParams = serde_json::from_value(params.clone()).unwrap_or_default();
    match p.action.as_deref() {
        Some("allow") => {
            let target = p.target.context("acl allow needs a target")?;
            let (pk, slug) = state.with_store(|s| resolve_acl_target(s, &target))?;
            crate::acl::allow(&pk, &slug)?;
            state.with_store(|s| {
                s.remove_pending_agent(&pk).ok();
            });
            // Newly-trusted author: refresh the union subscription.
            resubscribe(state).await.ok();
            Ok(serde_json::json!({ "slug": slug, "pubkey": pk }))
        }
        Some("block") => {
            let target = p.target.context("acl block needs a target")?;
            let (pk, slug) = state.with_store(|s| resolve_acl_target(s, &target))?;
            crate::acl::block(&pk, &slug)?;
            state.with_store(|s| {
                s.remove_pending_agent(&pk).ok();
            });
            Ok(serde_json::json!({ "slug": slug, "pubkey": pk }))
        }
        _ => {
            let pending = state.with_store(|s| s.list_pending_agents().unwrap_or_default());
            let allowed = crate::acl::allowed().len();
            let blocked = crate::acl::blocked().len();
            Ok(serde_json::json!({
                "pending": pending.iter().map(|p| serde_json::json!({"slug": p.slug, "pubkey": p.pubkey, "host": p.host})).collect::<Vec<_>>(),
                "allowed": allowed,
                "blocked": blocked,
            }))
        }
    }
}

fn resolve_acl_target(store: &Store, target: &str) -> Result<(String, String)> {
    if target.len() == 64 && target.chars().all(|c| c.is_ascii_hexdigit()) {
        let slug = store
            .list_pending_agents()?
            .into_iter()
            .find(|p| p.pubkey == target)
            .map(|p| p.slug)
            .unwrap_or_else(|| "agent".to_string());
        return Ok((target.to_string(), slug));
    }
    let m = store
        .list_pending_agents()?
        .into_iter()
        .find(|p| p.slug == target);
    match m {
        Some(p) => Ok((p.pubkey, p.slug)),
        None => anyhow::bail!(
            "no pending agent named {target:?}; use a pubkey or `tenex-edge acl list`"
        ),
    }
}

// ── doctor ───────────────────────────────────────────────────────────────────

pub(super) async fn rpc_doctor(state: &Arc<DaemonState>) -> Result<serde_json::Value> {
    use nostr_sdk::prelude::{Alphabet, EventBuilder, Filter, Kind, SingleLetterTag, Tag};
    let relays = state.cfg.relays.clone();
    let probe = state
        .keys_for(&state.hosted_pubkeys().first().cloned().unwrap_or_default())
        .map(|k| k.public_key().to_hex());
    let t = format!("te-doctor-{}", now_secs());
    let builder = EventBuilder::new(Kind::from(1u16), format!("tenex-edge doctor {t}"))
        .tags([Tag::parse(["h", &t])?]);
    // Sign with the daemon's connection key (any key works for the probe).
    let publish = match state.transport.publish_builder(builder).await {
        Ok(id) => format!("OK ({})", crate::util::pubkey_short(&id.to_hex())),
        Err(e) => format!("ERR {e:#}"),
    };
    tokio::time::sleep(Duration::from_secs(1)).await;
    let f = Filter::new()
        .kind(Kind::from(1u16))
        .custom_tag(SingleLetterTag::lowercase(Alphabet::H), &t)
        .limit(5);
    let readback = match state.transport.fetch(f, Duration::from_secs(5)).await {
        Ok(evs) => format!("{} event(s) with #h={t}", evs.len()),
        Err(e) => format!("ERR {e:#}"),
    };
    Ok(serde_json::json!({
        "relays": relays,
        "probe_pubkey": probe,
        "publish": publish,
        "readback": readback,
    }))
}

// ── project_list ─────────────────────────────────────────────────────────────

/// List NIP-29 groups: fetch all kind:39000 events from the relay (the relay
/// authors these events, so no author filter is used) and return the slug +
/// description for each. Results are also cached locally.
pub(super) async fn rpc_project_list(state: &Arc<DaemonState>) -> Result<serde_json::Value> {
    use nostr_sdk::prelude::{Filter, Kind};

    let filter = Filter::new().kind(Kind::from(39000u16)).limit(200);
    let events = state
        .transport
        .fetch(filter, Duration::from_secs(5))
        .await
        .unwrap_or_default();

    let now = now_secs();
    let mut projects: Vec<serde_json::Value> = Vec::new();
    for ev in &events {
        let Some(slug) = event_tag(ev, "d") else {
            continue;
        };
        let about = event_tag(ev, "about").unwrap_or("").to_string();
        state.with_store(|s| {
            s.upsert_project_meta(slug, &about, now).ok();
        });
        projects.push(serde_json::json!({ "slug": slug, "about": about }));
    }
    projects.sort_by(|a, b| {
        a["slug"]
            .as_str()
            .unwrap_or("")
            .cmp(b["slug"].as_str().unwrap_or(""))
    });

    Ok(serde_json::json!({ "projects": projects }))
}

// ── project_edit ─────────────────────────────────────────────────────────────

/// Publish a NIP-29 kind:9002 (edit-metadata) event signed by the human user's
/// nsec. The relay validates admin rights and updates its kind:39000 accordingly.
pub(super) async fn rpc_project_edit(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    use nostr_sdk::prelude::{EventBuilder, Keys, Kind, Tag};

    #[derive(serde::Deserialize)]
    struct P {
        project: String,
        description: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("project_edit params")?;

    let nsec = state
        .cfg
        .user_nsec
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("userNsec not set in ~/.tenex/config.json"))?;
    let user_keys = Keys::parse(nsec).context("parsing userNsec")?;

    // kind:9002 = NIP-29 edit-metadata. The relay validates admin rights and
    // re-publishes kind:39000 signed by the relay key.
    let builder = EventBuilder::new(Kind::from(9002u16), "").tags([
        Tag::parse(["d", &p.project])?,
        Tag::parse(["about", &p.description])?,
    ]);
    let event_id = state.transport.publish_signed(builder, &user_keys).await?;

    // Optimistically update local cache; the relay will also push kind:39000.
    let now = now_secs();
    state.with_store(|s| {
        s.upsert_project_meta(&p.project, &p.description, now).ok();
    });

    Ok(serde_json::json!({
        "event_id": event_id.to_hex(),
        "project": p.project,
    }))
}

// ── project_add ──────────────────────────────────────────────────────────────

/// Publish a NIP-29 kind:9000 (put-user) event to add a pubkey to the group.
/// Accepts hex, npub (bech32), or a NIP-05 address (user@domain.com).
pub(super) async fn rpc_project_add(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    use nostr_sdk::prelude::Keys;

    #[derive(serde::Deserialize)]
    struct P {
        project: String,
        pubkey: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("project_add params")?;

    let nsec = state
        .cfg
        .user_nsec
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("userNsec not set in ~/.tenex/config.json"))?;
    let user_keys = Keys::parse(nsec).context("parsing userNsec")?;

    let pubkey_hex = resolve_pubkey_hex(&p.pubkey).await?;

    let builder = crate::codec::kind1::group_put_user(&p.project, &pubkey_hex)?;
    state
        .transport
        .publish_signed_checked(builder, &user_keys)
        .await?;

    state.with_store(|s| {
        s.upsert_group_member(&p.project, &pubkey_hex, "member", now_secs())
            .ok();
    });

    Ok(serde_json::json!({
        "project": p.project,
        "pubkey": pubkey_hex,
    }))
}

async fn resolve_pubkey_hex(input: &str) -> Result<String> {
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
