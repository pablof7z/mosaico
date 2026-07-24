use crate::nmp_client::NmpRelayClient;
use nostr::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) const KIND_CREATE_GROUP: u16 = 9007;
pub(crate) const KIND_PUT_USER: u16 = 9000;
pub(crate) const KIND_GROUP_METADATA: u16 = 39000;
pub(crate) const KIND_GROUP_ADMINS: u16 = 39001;
pub(crate) const KIND_GROUP_MEMBERS: u16 = 39002;
pub(crate) const KIND_NOTE: u16 = 1;

pub(crate) fn relay_url() -> String {
    std::env::var("MOSAICO_NIP29_RELAY")
        .expect("set MOSAICO_NIP29_RELAY to the explicit relay used for this public probe")
}

pub(crate) fn unique_slug() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("mosaico-probe-{nanos:x}")
}

pub(crate) fn h_tag(slug: &str) -> Tag {
    Tag::custom(
        TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::H)),
        [slug],
    )
}

pub(crate) async fn connect(keys: Keys, relay: &str) -> NmpRelayClient {
    NmpRelayClient::connect(keys, relay)
        .await
        .expect("connect NMP relay client")
}

/// Publish a pre-signed event over `client`; report and return whether the relay
/// accepted it (best-effort: inspect Output, fall back to readback by caller).
pub(crate) async fn publish(client: &NmpRelayClient, signed: &Event, label: &str) -> bool {
    match client.send_event(signed).await {
        Ok(out) => {
            let ok = !out.success.is_empty();
            eprintln!(
                "[probe] {label}: send_event success={:?} failed={:?}",
                out.success, out.failed
            );
            ok
        }
        Err(e) => {
            eprintln!("[probe] {label}: send_event ERROR {e}");
            false
        }
    }
}

pub(crate) async fn fetch(client: &NmpRelayClient, filter: Filter, label: &str) -> Vec<Event> {
    let evs = client
        .fetch_events(filter, Duration::from_secs(5))
        .await
        .map(|e| e.into_iter().collect::<Vec<_>>())
        .unwrap_or_default();
    eprintln!("[probe] {label}: fetched {} event(s)", evs.len());
    for e in &evs {
        eprintln!(
            "[probe]   kind={} pubkey={} tags={:?} content={:?}",
            e.kind.as_u16(),
            &e.pubkey.to_hex()[..8],
            e.tags
                .iter()
                .map(|t| t.as_slice().to_vec())
                .collect::<Vec<_>>(),
            e.content
        );
    }
    evs
}

/// Create a NIP-29 group with a client-chosen h-tag id. Retries rate limits;
/// returns false when the relay stays throttled so the probe can skip.
pub(crate) async fn create_group_with_retry(
    admin: &Keys,
    admin_c: &NmpRelayClient,
    slug: &str,
) -> bool {
    for (attempt, backoff_s) in [2u64, 5, 12].into_iter().enumerate() {
        let create = EventBuilder::new(Kind::from(KIND_CREATE_GROUP), "")
            .tags([h_tag(slug)])
            .sign_with_keys(admin)
            .expect("sign create");
        match admin_c.send_event(&create).await {
            Ok(out) if !out.success.is_empty() => {
                eprintln!("[probe] 9007 create-group: success={:?}", out.success);
                return true;
            }
            Ok(out) if out.failed.values().any(|m| m.contains("rate-limited")) => {
                eprintln!(
                    "[probe] 9007 create-group rate-limited (attempt {}); backing off {backoff_s}s",
                    attempt + 1
                );
                tokio::time::sleep(Duration::from_secs(backoff_s)).await;
            }
            Ok(out) => {
                eprintln!("[probe] 9007 create-group failed={:?}", out.failed);
                return false;
            }
            Err(e) => {
                eprintln!("[probe] 9007 create-group ERROR {e}");
                return false;
            }
        }
    }
    false
}

pub(crate) async fn group_id_honored(
    admin_c: &NmpRelayClient,
    slug: &str,
    created_ok: bool,
) -> bool {
    let meta = fetch(
        admin_c,
        Filter::new()
            .kind(Kind::from(KIND_GROUP_METADATA))
            .identifier(slug),
        "39000 metadata (#d=slug)",
    )
    .await;
    let admins = fetch(
        admin_c,
        Filter::new()
            .kind(Kind::from(KIND_GROUP_ADMINS))
            .identifier(slug),
        "39001 admins (#d=slug)",
    )
    .await;
    let id_honored = !meta.is_empty() || !admins.is_empty();
    eprintln!(
        "[probe] Q1 client-chosen id honored: {} (created_ok={created_ok})",
        if id_honored { "YES" } else { "NO / UNKNOWN" }
    );
    id_honored
}
