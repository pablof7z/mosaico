//! SEED (ignored by default; run explicitly against the live NIP-29 relay):
//!
//!   TE_NIP29_RELAY=wss://nip29.f7z.io \
//!     cargo test --test seed_validation -- --ignored --nocapture
//!
//! Publishes ONE complete, self-contained agent session to the live relay so a
//! reader app (tenex-off) can be validated end-to-end: a kind:30315 status that
//! carries the NEW NIP-10 `["e", root, "", "root"]` thread-root link, plus the
//! kind:1 conversation it points at (the user's root prompt + the agent's two
//! turn replies + a follow-up prompt). Everything is built through the REAL
//! `Nip29WireCodec`, so this also proves the patched wire format.
//!
//! A fresh NIP-29 group is OPEN by default (writes accepted, non-members may
//! read), per tests/nip29_probe.rs findings — so we create a unique group and
//! publish into it without locking. The reader subscribes by kind only, so it
//! picks the session up regardless of membership.
//!
//! Prints the group slug, session id, agent npub, title and the seeded bodies so
//! the simulator validation can locate the session and assert its messages.
//! This is a manual reader-app seed, not a routine regression test; it leaves a
//! validation group, status, and conversation events on the configured relay.

use nostr_sdk::prelude::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tenex_edge::domain::{AgentRef, DomainEvent, Profile as TeProfile, Status};
use tenex_edge::fabric::nip29::wire::Nip29WireCodec;

fn relay_url() -> String {
    std::env::var("TE_NIP29_RELAY").unwrap_or_else(|_| "wss://nip29.f7z.io".to_string())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

async fn connect(keys: Keys, relay: &str) -> Client {
    let opts = ClientOptions::default().automatic_authentication(true);
    let client = Client::builder().signer(keys).opts(opts).build();
    client.add_relay(relay).await.expect("add relay");
    client.connect().await;
    client.wait_for_connection(Duration::from_secs(8)).await;
    // NIP-42 warm-up: force AUTH before any REQ/EVENT (relay29 is auth-gated).
    let _ = client
        .fetch_events(
            Filter::new().kind(Kind::from(0u16)).limit(1),
            Duration::from_secs(5),
        )
        .await;
    client
}

/// Build a signed event from a domain event through the real codec, stamping an
/// explicit created_at so the conversation orders deterministically.
async fn sign_domain(keys: &Keys, ev: &DomainEvent, created_at: u64) -> Event {
    let builder = Nip29WireCodec
        .encode_event(ev)
        .expect("encode")
        .custom_created_at(Timestamp::from_secs(created_at));
    let unsigned = builder.build(keys.public_key());
    keys.sign_event(unsigned).await.expect("sign")
}

async fn publish(client: &Client, signed: &Event, label: &str) {
    match client.send_event(signed).await {
        Ok(out) => eprintln!(
            "[seed] {label}: id={} success={:?} failed={:?}",
            &signed.id.to_hex()[..12],
            out.success,
            out.failed
        ),
        Err(e) => eprintln!("[seed] {label}: send_event ERROR {e}"),
    }
}

#[tokio::test]
#[ignore]
async fn seed_session_with_thread_root_link() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let relay = relay_url();

    let admin = Keys::generate();
    let agent = Keys::generate();
    let user = Keys::generate();
    let agent_pk = agent.public_key().to_hex();
    let user_pk = user.public_key().to_hex();

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let channel = format!("tenex-off-val-{nanos:x}");
    let session_id = format!("val-sess-{nanos:x}");
    let title = "VALIDATION: session→thread link";

    eprintln!("\n[seed] ===== seed session with thread-root link =====");
    eprintln!("[seed] relay      = {relay}");
    eprintln!("[seed] channel(h) = {channel}");
    eprintln!("[seed] session-id = {session_id}");
    eprintln!("[seed] title      = {title}");
    eprintln!(
        "[seed] agent npub  = {}",
        agent.public_key().to_bech32().unwrap_or_default()
    );

    let admin_c = connect(admin.clone(), &relay).await;
    let agent_c = connect(agent.clone(), &relay).await;
    let user_c = connect(user.clone(), &relay).await;

    // ── Create an OPEN group with our chosen id (h == channel). Retry rate limits.
    let mut created = false;
    for (attempt, backoff) in [2u64, 5, 12].into_iter().enumerate() {
        let create = EventBuilder::new(Kind::from(9007u16), "")
            .tags([Tag::custom(
                TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::H)),
                [channel.clone()],
            )])
            .build(admin.public_key());
        let create = admin.sign_event(create).await.expect("sign create");
        match admin_c.send_event(&create).await {
            Ok(out) if !out.success.is_empty() => {
                eprintln!("[seed] 9007 create-group: ok");
                created = true;
                break;
            }
            Ok(out) if out.failed.values().any(|m| m.contains("rate-limited")) => {
                eprintln!(
                    "[seed] create rate-limited (attempt {}); backoff {backoff}s",
                    attempt + 1
                );
                tokio::time::sleep(Duration::from_secs(backoff)).await;
            }
            Ok(out) => {
                eprintln!("[seed] create failed={:?}", out.failed);
                break;
            }
            Err(e) => {
                eprintln!("[seed] create ERROR {e}");
                break;
            }
        }
    }
    assert!(
        created,
        "could not create group (relay rate-limited?) — rerun"
    );
    tokio::time::sleep(Duration::from_millis(800)).await;

    let agent_ref = AgentRef::new(agent_pk.clone(), "validator");
    let base = now_secs();

    // 1) Agent profile (kind:0) so the reader shows a name, not a raw npub.
    let profile = sign_domain(
        &agent,
        &DomainEvent::Profile(TeProfile {
            agent: agent_ref.clone(),
            agent_slug: "validator".into(),
            host: "seed-host".into(),
            owners: vec![user_pk.clone()],
            is_backend: false,
        }),
        base,
    )
    .await;
    publish(&agent_c, &profile, "kind:0 agent profile").await;

    // 2) kind:30315 status. Far-future
    //    expiration so the session reads as live in the reader.
    let status = sign_domain(
        &agent,
        &DomainEvent::Status(Status {
            agent: agent_ref.clone(),
            channels: vec![channel.clone()],
            session_id: session_id.clone().into(),
            host: "seed-host".into(),
            title: title.into(),
            activity: String::new(),
            busy: false,
            rel_cwd: "tenex-off".into(),
            expires_at: Some(base + 365 * 24 * 3600),
        }),
        base + 1,
    )
    .await;
    publish(&agent_c, &status, "kind:30315 status").await;

    // ── Read back the status to confirm it's retrievable.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let statuses = admin_c
        .fetch_events(
            Filter::new()
                .kind(Kind::from(30315u16))
                .custom_tag(SingleLetterTag::lowercase(Alphabet::H), &channel),
            Duration::from_secs(5),
        )
        .await
        .map(|e| e.into_iter().collect::<Vec<_>>())
        .unwrap_or_default();

    eprintln!("[seed] readback: {} status event(s)", statuses.len());

    eprintln!("\n[seed] ===== SEED COMPLETE — open tenex-off and find this session =====");
    eprintln!("[seed] Look for the session titled: {title:?}");

    assert!(!statuses.is_empty(), "status must be retrievable");

    admin_c.disconnect().await;
    agent_c.disconnect().await;
    user_c.disconnect().await;
}
