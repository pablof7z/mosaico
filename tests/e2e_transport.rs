//! End-to-end: publish every domain event through the real transport to a real
//! relay, and verify a subscriber decodes them back. Exercises NIP-29 wire
//! encoding + transport + a live relay together.

#[path = "common/mod.rs"]
mod common;

use common::TestRelay;
use nostr_sdk::prelude::{Keys, RelayPoolNotification};
use std::time::Duration;
use tenex_edge::domain::{AgentRef, DomainEvent, Profile, Status};
use tenex_edge::fabric::nip29::wire::Nip29WireCodec;
use tenex_edge::fabric::nostr_delivery::scope_filters;
use tenex_edge::fabric::Scope;
use tenex_edge::transport::Transport;

#[tokio::test]
async fn publishes_and_decodes_all_event_types() {
    let relay = TestRelay::start();
    let codec = Nip29WireCodec;

    let agent_keys = Keys::generate();
    let reader_keys = Keys::generate();
    let agent_pk = agent_keys.public_key().to_hex();
    let reader_pk = reader_keys.public_key().to_hex();
    let project = "tenex-edge".to_string();

    // Reader subscribes FIRST (presence is ephemeral — must be listening live).
    let reader = Transport::connect(std::slice::from_ref(&relay.url), reader_keys)
        .await
        .expect("reader connects");
    let scope = Scope {
        authors: vec![agent_pk.clone()],
        project: Some(project.clone()),
    };
    reader
        .subscribe(scope_filters(&scope))
        .await
        .expect("subscribe");
    let mut notifications = reader.notifications();
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Agent connects and publishes one of each.
    let agent = Transport::connect(std::slice::from_ref(&relay.url), agent_keys)
        .await
        .expect("agent connects");
    let aref = AgentRef::new(agent_pk.clone(), "coder");

    let events = vec![
        DomainEvent::Profile(Profile {
            agent: aref.clone(),
            host: "test-host".into(),
            owners: vec![reader_pk.clone()],
            is_backend: false,
        }),
        DomainEvent::Status(Status {
            agent: aref.clone(),
            channels: vec![project.clone()],
            session_id: "sess-1".into(),
            host: "test-host".into(),
            title: "fixing the auth bug".into(),
            activity: "reading the diff".into(),
            busy: true,
            rel_cwd: String::new(),
            expires_at: Some(1_900_000_000),
        }),
    ];
    for ev in &events {
        let builder = codec.encode_event(ev).expect("encode");
        agent.publish_builder(builder).await.expect("publish");
    }

    // Collect decoded events for a few seconds.
    let mut seen: Vec<DomainEvent> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while seen.len() < 2 && tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), notifications.recv()).await {
            Ok(Ok(RelayPoolNotification::Event { event, .. })) => {
                if let Some(de) = codec.decode_event(&event) {
                    if !seen.contains(&de) {
                        seen.push(de);
                    }
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) => break,
            Err(_) => {} // timeout tick; loop
        }
    }

    // Identify the status by its title; the decoded status also carries its
    // session id, but the title is the stable user-facing session summary.
    let has_status = seen
        .iter()
        .any(|e| matches!(e, DomainEvent::Status(s) if s.title == "fixing the auth bug"));
    let has_profile = seen
        .iter()
        .any(|e| matches!(e, DomainEvent::Profile(p) if p.host == "test-host"));

    assert!(has_status, "expected status; saw {seen:#?}");
    assert!(has_profile, "expected profile; saw {seen:#?}");
}
