//! SEED (ignored by default; run explicitly against the live NIP-29 relay):
//!
//!   cargo test --test seed_validation -- --ignored --nocapture
//!
//! Publishes ONE complete, self-contained agent session to the live relay so a
//! reader app (tenex-off) can be validated end-to-end: a kind:30315 status that
//! carries the NEW NIP-10 `["e", root, "", "root"]` thread-root link, plus the
//! kind:1 conversation it points at (the user's root prompt + the agent's two
//! turn replies + a follow-up prompt). Everything is built through the REAL
//! `Kind1Codec`, so this also proves the patched wire format.
//!
//! A fresh NIP-29 group is OPEN by default (writes accepted, non-members may
//! read), per tests/nip29_probe.rs findings — so we create a unique group and
//! publish into it without locking. The reader subscribes by kind only, so it
//! picks the session up regardless of membership.
//!
//! Prints the group slug, session id, agent npub, title and the seeded bodies so
//! the simulator validation can locate the session and assert its messages.

use nostr_sdk::prelude::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tenex_edge::codec::{Codec, Kind1Codec};
use tenex_edge::domain::{
    AgentRef, DomainEvent, Mention, MentionMeta, Profile as TeProfile, Status, TurnReply,
};

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
    let builder = Kind1Codec
        .encode(ev)
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
    let project = format!("tenex-off-val-{nanos:x}");
    let session_id = format!("val-sess-{nanos:x}");
    let title = "VALIDATION: session→thread link";

    eprintln!("\n[seed] ===== seed session with thread-root link =====");
    eprintln!("[seed] relay      = {relay}");
    eprintln!("[seed] project(h) = {project}");
    eprintln!("[seed] session-id = {session_id}");
    eprintln!("[seed] title      = {title}");
    eprintln!(
        "[seed] agent npub  = {}",
        agent.public_key().to_bech32().unwrap_or_default()
    );

    let admin_c = connect(admin.clone(), &relay).await;
    let agent_c = connect(agent.clone(), &relay).await;
    let user_c = connect(user.clone(), &relay).await;

    // ── Create an OPEN group with our chosen id (h == project). Retry rate limits.
    let mut created = false;
    for (attempt, backoff) in [2u64, 5, 12].into_iter().enumerate() {
        let create = EventBuilder::new(Kind::from(9007u16), "")
            .tags([Tag::custom(
                TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::H)),
                [project.clone()],
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
                eprintln!("[seed] create rate-limited (attempt {}); backoff {backoff}s", attempt + 1);
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
    assert!(created, "could not create group (relay rate-limited?) — rerun");
    tokio::time::sleep(Duration::from_millis(800)).await;

    let agent_ref = AgentRef::new(agent_pk.clone(), "validator");
    let base = now_secs();

    // 1) Agent profile (kind:0) so the reader shows a name, not a raw npub.
    let profile = sign_domain(
        &agent,
        &DomainEvent::Profile(TeProfile {
            agent: agent_ref.clone(),
            host: "seed-host".into(),
            owners: vec![user_pk.clone()],
        }),
        base,
    )
    .await;
    publish(&agent_c, &profile, "kind:0 agent profile").await;

    // 2) Root user prompt (kind:1 Mention, no `e` tag → the thread root/OP).
    let prompt1 = sign_domain(
        &user,
        &DomainEvent::Mention(Mention {
            from: AgentRef::new(user_pk.clone(), String::new()),
            to_pubkey: agent_pk.clone(),
            project: project.clone(),
            body: "Seed prompt one: please summarize the session→thread fix.".into(),
            meta: MentionMeta::default(),
        }),
        base + 1,
    )
    .await;
    let root_id = prompt1.id.to_hex();
    publish(&user_c, &prompt1, "kind:1 root prompt (OP)").await;

    // 3) Agent turn reply (kind:1, e-root=OP) — first answer.
    let reply1 = sign_domain(
        &agent,
        &DomainEvent::TurnReply(TurnReply {
            agent: agent_ref.clone(),
            project: project.clone(),
            body: "Seed reply one: the kind:30315 now carries the thread root as an e-tag."
                .into(),
            root_event_id: root_id.clone(),
            reply_event_id: root_id.clone(),
        }),
        base + 2,
    )
    .await;
    publish(&agent_c, &reply1, "kind:1 turn reply #1").await;

    // 4) Follow-up user prompt (its own orphan kind:1 root, no `e` tag).
    let prompt2 = sign_domain(
        &user,
        &DomainEvent::Mention(Mention {
            from: AgentRef::new(user_pk.clone(), String::new()),
            to_pubkey: agent_pk.clone(),
            project: project.clone(),
            body: "Seed prompt two: and how does the reader reconstruct it?".into(),
            meta: MentionMeta::default(),
        }),
        base + 3,
    )
    .await;
    let prompt2_id = prompt2.id.to_hex();
    publish(&user_c, &prompt2, "kind:1 follow-up prompt").await;

    // 5) Agent turn reply (kind:1, e-root=OP, e-reply=prompt2) — second answer.
    let reply2 = sign_domain(
        &agent,
        &DomainEvent::TurnReply(TurnReply {
            agent: agent_ref.clone(),
            project: project.clone(),
            body: "Seed reply two: it maps session→root via the 30315 e-tag, then \
                   joins the root prompt with all e-root replies."
                .into(),
            root_event_id: root_id.clone(),
            reply_event_id: prompt2_id.clone(),
        }),
        base + 4,
    )
    .await;
    publish(&agent_c, &reply2, "kind:1 turn reply #2").await;

    // 6) THE LINK: kind:30315 status carrying the thread-root e-tag. Far-future
    //    expiration so the session reads as live in the reader.
    let status = sign_domain(
        &agent,
        &DomainEvent::Status(Status {
            agent: agent_ref.clone(),
            project: project.clone(),
            session_id: session_id.clone().into(),
            host: "seed-host".into(),
            title: title.into(),
            activity: String::new(),
            busy: false,
            rel_cwd: "tenex-off".into(),
            expires_at: Some(base + 365 * 24 * 3600),
            thread_root_id: Some(root_id.clone()),
        }),
        base + 5,
    )
    .await;
    publish(&agent_c, &status, "kind:30315 status (with thread-root e-tag)").await;

    // ── Read back the status and confirm the link survived the round trip.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let statuses = admin_c
        .fetch_events(
            Filter::new()
                .kind(Kind::from(30315u16))
                .custom_tag(SingleLetterTag::lowercase(Alphabet::H), &project),
            Duration::from_secs(5),
        )
        .await
        .map(|e| e.into_iter().collect::<Vec<_>>())
        .unwrap_or_default();

    let linked = statuses.iter().any(|e| {
        e.tags.iter().any(|t| {
            let s = t.as_slice();
            s.first().map(String::as_str) == Some("e") && s.get(1).map(String::as_str) == Some(&root_id)
        })
    });
    eprintln!(
        "[seed] readback: {} status event(s); thread-root e-tag present = {linked}",
        statuses.len()
    );

    let notes = admin_c
        .fetch_events(
            Filter::new()
                .kind(Kind::from(1u16))
                .custom_tag(SingleLetterTag::lowercase(Alphabet::H), &project),
            Duration::from_secs(5),
        )
        .await
        .map(|e| e.into_iter().collect::<Vec<_>>())
        .unwrap_or_default();
    eprintln!("[seed] readback: {} kind:1 event(s) in the conversation", notes.len());

    eprintln!("\n[seed] ===== SEED COMPLETE — open tenex-off and find this session =====");
    eprintln!("[seed] Look for the session titled: {title:?}");
    eprintln!("[seed] Tapping it must show 4 messages (2 prompts, 2 replies), oldest→newest:");
    eprintln!("[seed]   1. (user)  Seed prompt one ...");
    eprintln!("[seed]   2. (agent) Seed reply one ...");
    eprintln!("[seed]   3. (user)  Seed prompt two ...");
    eprintln!("[seed]   4. (agent) Seed reply two ...");

    assert!(linked, "the kind:30315 must carry the thread-root e-tag on readback");
    assert!(statuses.iter().count() >= 1, "status must be retrievable");
    assert!(notes.len() >= 4, "all four conversation notes must be retrievable");

    admin_c.disconnect().await;
    agent_c.disconnect().await;
    user_c.disconnect().await;
}
