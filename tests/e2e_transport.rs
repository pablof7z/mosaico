//! End-to-end: publish every domain event through the real transport to a real
//! relay, and verify a subscriber decodes them back. Exercises codec + transport
//! + a live relay together.

mod common;

use common::TestRelay;
use nostr_sdk::prelude::{Keys, RelayPoolNotification};
use std::time::Duration;
use tenex_edge::codec::{Codec, Kind1Codec};
use tenex_edge::domain::*;
use tenex_edge::fabric::nostr_delivery::scope_filters;
use tenex_edge::fabric::Scope;
use tenex_edge::transport::Transport;

#[tokio::test]
async fn publishes_and_decodes_all_event_types() {
    let relay = TestRelay::start();
    let codec = Kind1Codec;

    let agent_keys = Keys::generate();
    let reader_keys = Keys::generate();
    let agent_pk = agent_keys.public_key().to_hex();
    let reader_pk = reader_keys.public_key().to_hex();
    let project = "tenex-edge".to_string();

    // Reader subscribes FIRST (presence is ephemeral — must be listening live).
    let reader = Transport::connect(&[relay.url.clone()], reader_keys)
        .await
        .expect("reader connects");
    let scope = Scope {
        authors: vec![agent_pk.clone()],
        project: Some(project.clone()),
        mentions_to: Some(reader_pk.clone()),
        owners: Vec::new(),
        thread: None,
    };
    reader
        .subscribe(scope_filters(&scope))
        .await
        .expect("subscribe");
    let mut notifications = reader.notifications();
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Agent connects and publishes one of each.
    let agent = Transport::connect(&[relay.url.clone()], agent_keys)
        .await
        .expect("agent connects");
    let aref = AgentRef::new(agent_pk.clone(), "coder");

    let events = vec![
        DomainEvent::Profile(Profile {
            agent: aref.clone(),
            host: "test-host".into(),
            owners: vec![reader_pk.clone()],
        }),
        DomainEvent::Status(Status {
            agent: aref.clone(),
            project: project.clone(),
            session_id: "sess-1".into(),
            host: "test-host".into(),
            title: "fixing the auth bug".into(),
            activity: "reading the diff".into(),
            busy: true,
            rel_cwd: String::new(),
            expires_at: Some(1_900_000_000),
            thread_root_id: None,
        }),
        DomainEvent::Activity(Activity {
            agent: aref.clone(),
            project: project.clone(),
            text: "fixing the auth bug".into(),
        }),
        DomainEvent::Mention(Mention {
            from: aref.clone(),
            to_pubkey: reader_pk.clone(),
            project: project.clone(),
            body: "can you review?".into(),
            // Stage 4: target_session and from_session are no longer wire fields.
            // Routing is by session pubkey via the p-tag (to_pubkey).
            meta: tenex_edge::domain::MentionMeta::default(),
        }),
    ];
    for ev in &events {
        let builder = codec.encode(ev).expect("encode");
        agent.publish_builder(builder).await.expect("publish");
    }

    // Collect decoded events for a few seconds.
    let mut seen: Vec<DomainEvent> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while seen.len() < 4 && tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), notifications.recv()).await {
            Ok(Ok(RelayPoolNotification::Event { event, .. })) => {
                if let Some(de) = codec.decode(&event) {
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

    let has_status = seen
        .iter()
        .any(|e| matches!(e, DomainEvent::Status(s) if s.session_id.as_str() == "sess-1"));
    let has_activity = seen
        .iter()
        .any(|e| matches!(e, DomainEvent::Activity(a) if a.text == "fixing the auth bug"));
    let has_profile = seen
        .iter()
        .any(|e| matches!(e, DomainEvent::Profile(p) if p.host == "test-host"));
    // Stage 4: target_session removed from domain. Assert routing by to_pubkey only
    // (the session pubkey is the wire-level address; target_session no longer exists).
    let has_mention = seen.iter().any(|e| matches!(e, DomainEvent::Mention(m) if m.to_pubkey == reader_pk && m.body == "can you review?"));

    assert!(has_status, "expected status; saw {seen:#?}");
    assert!(has_activity, "expected activity; saw {seen:#?}");
    assert!(has_profile, "expected profile; saw {seen:#?}");
    assert!(has_mention, "expected mention; saw {seen:#?}");
}

/// AC5: A Mention's `to_pubkey` MUST be encoded as a `["p", to_pubkey]` tag
/// on the wire. The old `from-session` envelope tag MUST NOT appear (Stage 4
/// removed it). This is a pure codec unit test — no relay required.
#[test]
fn ac5_mention_encodes_p_tag_for_session_pubkey_no_from_session() {
    use nostr_sdk::prelude::Keys;
    let agent_keys = Keys::generate();
    // Simulate a session pubkey (could be any valid pubkey — the codec just
    // stamps whatever to_pubkey it receives into the p-tag).
    let session_pk = Keys::generate().public_key().to_hex();

    let mention = DomainEvent::Mention(Mention {
        from: AgentRef::new(agent_keys.public_key().to_hex(), "coder".to_string()),
        to_pubkey: session_pk.clone(),
        project: "myproject".to_string(),
        body: "hello, targeted session".to_string(),
        meta: MentionMeta::default(),
    });

    let codec = Kind1Codec;
    let builder = codec.encode(&mention).expect("encode Mention");
    // Build unsigned to inspect tags without async signing.
    let unsigned = builder.build(agent_keys.public_key());

    // Must carry ["p", session_pk] — the session pubkey is the wire address.
    let has_p_tag = unsigned.tags.iter().any(|t| {
        let s = t.as_slice();
        s.first().map(String::as_str) == Some("p")
            && s.get(1).map(String::as_str) == Some(session_pk.as_str())
    });
    assert!(
        has_p_tag,
        "AC5: encoded Mention must carry [\"p\", to_pubkey] tag; got tags: {:?}",
        unsigned.tags.iter().map(|t| t.as_slice().to_vec()).collect::<Vec<_>>()
    );

    // Must NOT carry a "from-session" tag (Stage 4 removed the sender-session
    // envelope from the wire — routing is solely by the p-tagged session pubkey).
    let has_from_session = unsigned.tags.iter().any(|t| {
        t.as_slice().first().map(String::as_str) == Some("from-session")
    });
    assert!(
        !has_from_session,
        "AC5: Stage 4 Mention must NOT carry a \"from-session\" tag on the wire"
    );

    // Must NOT carry a "session-id" (target_session) wire tag either.
    let has_session_id_tag = unsigned.tags.iter().any(|t| {
        t.as_slice().first().map(String::as_str) == Some("session-id")
    });
    assert!(
        !has_session_id_tag,
        "AC5: Stage 4 Mention must NOT carry a wire \"session-id\" target_session tag"
    );
}
