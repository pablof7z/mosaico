use super::*;

fn roundtrip(ev: DomainEvent, keys: &Keys) -> DomainEvent {
    let codec = Kind1Codec;
    let builder = codec.encode(&ev).expect("encode");
    let signed = builder.sign_with_keys(keys).expect("sign");
    codec.decode(&signed).expect("decode")
}

fn agent(keys: &Keys, slug: &str) -> AgentRef {
    AgentRef::new(keys.public_key().to_hex(), slug)
}

fn has_tag(event: &Event, name: &str, value: &str) -> bool {
    event.tags.iter().any(|t| {
        let s = t.as_slice();
        s.first().map(String::as_str) == Some(name) && s.get(1).map(String::as_str) == Some(value)
    })
}

fn has_tag_name(event: &Event, name: &str) -> bool {
    event
        .tags
        .iter()
        .any(|t| t.as_slice().first().map(String::as_str) == Some(name))
}

#[test]
fn profile_roundtrip() {
    let keys = Keys::generate();
    let ev = DomainEvent::Profile(Profile {
        agent: agent(&keys, "coder"),
        host: "pablos' laptop".into(),
        owners: vec!["09d4".repeat(16)],
    });
    assert_eq!(roundtrip(ev.clone(), &keys), ev);
}

#[test]
fn presence_roundtrip() {
    let keys = Keys::generate();
    let ev = DomainEvent::Presence(Presence {
        agent: agent(&keys, "coder"),
        project: "tenex-edge".into(),
        session_id: "sess-123".into(),
        host: "laptop".into(),
        rel_cwd: String::new(),
        audience: vec!["aa".repeat(32), "bb".repeat(32)],
        expires_at: 1_900_000_000,
    });
    assert_eq!(roundtrip(ev.clone(), &keys), ev);
}

#[test]
fn presence_rel_cwd_roundtrips_and_emits_tag() {
    let keys = Keys::generate();
    let ev = DomainEvent::Presence(Presence {
        agent: agent(&keys, "coder"),
        project: "tenex-edge".into(),
        session_id: "sess-123".into(),
        host: "laptop".into(),
        rel_cwd: "worktree1".into(),
        audience: vec!["aa".repeat(32)],
        expires_at: 1_900_000_000,
    });
    // The relative dir survives encode→decode …
    assert_eq!(roundtrip(ev.clone(), &keys), ev);
    // … and lands as a `rel-cwd` tag on the wire.
    let signed = Kind1Codec
        .encode(&ev)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    assert!(has_tag(&signed, "rel-cwd", "worktree1"));
}

#[test]
fn empty_rel_cwd_emits_no_tag_and_decodes_empty() {
    // Wire compat: events without a rel-cwd tag (old peers) decode to "".
    let keys = Keys::generate();
    let ev = DomainEvent::Presence(Presence {
        agent: agent(&keys, "coder"),
        project: "tenex-edge".into(),
        session_id: "sess-1".into(),
        host: "laptop".into(),
        rel_cwd: String::new(),
        audience: vec![],
        expires_at: 1_900_000_000,
    });
    let signed = Kind1Codec
        .encode(&ev)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    assert!(!has_tag_name(&signed, "rel-cwd"));
    match Kind1Codec.decode(&signed) {
        Some(DomainEvent::Presence(p)) => assert_eq!(p.rel_cwd, ""),
        other => panic!("expected presence, got {other:?}"),
    }
}

#[test]
fn presence_uses_session_scoped_nip38_heartbeat() {
    let keys = Keys::generate();
    let ev = DomainEvent::Presence(Presence {
        agent: agent(&keys, "coder"),
        project: "tenex-edge".into(),
        session_id: "sess-123".into(),
        host: "laptop".into(),
        rel_cwd: String::new(),
        audience: vec!["aa".repeat(32)],
        expires_at: 1_900_000_000,
    });
    let signed = Kind1Codec
        .encode(&ev)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    assert_eq!(signed.kind.as_u16(), KIND_PRESENCE);
    assert!(has_tag(&signed, "h", "tenex-edge"));
    assert!(has_tag(&signed, "d", "tenex-edge-presence:sess-123"));
    assert!(has_tag(&signed, "session-id", "sess-123"));
    assert!(has_tag(&signed, "expiration", "1900000000"));
}

#[test]
fn activity_roundtrip() {
    let keys = Keys::generate();
    let ev = DomainEvent::Activity(Activity {
        agent: agent(&keys, ""), // slug not on wire; resolved from profile store at routing time
        project: "tenex-edge".into(),
        text: "fixing the auth bug".into(),
    });
    assert_eq!(roundtrip(ev.clone(), &keys), ev);
}

#[test]
fn activity_uses_nip29_h_tag_not_hashtag() {
    let keys = Keys::generate();
    let ev = DomainEvent::Activity(Activity {
        agent: agent(&keys, "coder"),
        project: "tenex-edge".into(),
        text: "fixing the auth bug".into(),
    });
    let signed = Kind1Codec
        .encode(&ev)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    assert!(has_tag(&signed, "h", "tenex-edge"));
    assert!(!has_tag_name(&signed, "t"));
}

#[test]
fn status_roundtrip_with_expiry() {
    let keys = Keys::generate();
    let ev = DomainEvent::Status(Status {
        agent: agent(&keys, "coder"),
        project: "tenex-edge".into(),
        session_id: Some("sess-123".into()),
        text: "reviewing PR".into(),
        rel_cwd: String::new(),
        expires_at: Some(1_900_000_000),
    });
    assert_eq!(roundtrip(ev.clone(), &keys), ev);
    let signed = Kind1Codec
        .encode(&ev)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    assert!(has_tag(&signed, "session-id", "sess-123"));
}

#[test]
fn mention_roundtrip_session_targeted() {
    let keys = Keys::generate();
    let ev = DomainEvent::Mention(Mention {
        from: agent(&keys, ""), // slug not on wire; resolved from profile store at routing time
        to_pubkey: "cc".repeat(32),
        project: "tenex-edge".into(),
        body: "can you review?".into(),
        target_session: Some("sess-xyz".into()),
        // Distinct from target_session so the roundtrip proves they don't swap.
        from_session: Some("sender-sess-1".into()),
    });
    assert_eq!(roundtrip(ev.clone(), &keys), ev);
}

#[test]
fn mention_emits_from_session_tag_and_back_compat_decodes_none() {
    let keys = Keys::generate();
    // With a sender session → a `from-session` tag rides the wire.
    let ev = DomainEvent::Mention(Mention {
        from: agent(&keys, ""), // slug not on wire
        to_pubkey: "cc".repeat(32),
        project: "tenex-edge".into(),
        body: "ping".into(),
        target_session: None,
        from_session: Some("sender-9".into()),
    });
    let signed = Kind1Codec
        .encode(&ev)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    assert!(has_tag(&signed, "from-session", "sender-9"));

    // An old-peer note WITHOUT the tag decodes to `from_session: None`.
    let legacy = EventBuilder::new(Kind::from(KIND_NOTE), "ping")
        .tags([
            tag(&["h", "tenex-edge"]).unwrap(),
            tag(&["p", &"cc".repeat(32)]).unwrap(),
            tag(&["agent", &keys.public_key().to_hex(), "coder"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    match Kind1Codec.decode(&legacy) {
        Some(DomainEvent::Mention(m)) => assert_eq!(m.from_session, None),
        other => panic!("expected mention, got {other:?}"),
    }
}

#[test]
fn mention_uses_nip29_h_tag_not_hashtag() {
    let keys = Keys::generate();
    let ev = DomainEvent::Mention(Mention {
        from: agent(&keys, ""),
        to_pubkey: "cc".repeat(32),
        project: "tenex-edge".into(),
        body: "can you review?".into(),
        target_session: Some("sess-xyz".into()),
        from_session: None,
    });
    let signed = Kind1Codec
        .encode(&ev)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    assert!(has_tag(&signed, "h", "tenex-edge"));
    assert!(!has_tag_name(&signed, "t"));
}

#[test]
fn mention_to_self_keeps_p_tag() {
    // A mention from one session of an agent to another session of the SAME
    // agent has to_pubkey == the signer's own pubkey. Ensure the p tag survives.
    let keys = Keys::generate();
    let pk = keys.public_key().to_hex();
    let ev = DomainEvent::Mention(Mention {
        from: AgentRef::new(pk.clone(), ""), // slug not on wire
        to_pubkey: pk.clone(),
        project: "p".into(),
        body: "hi".into(),
        target_session: Some("s2".into()),
        from_session: Some("s1".into()),
    });
    let signed = Kind1Codec
        .encode(&ev)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    let has_p = signed
        .tags
        .iter()
        .any(|t| t.as_slice().first().map(|s| s.as_str()) == Some("p"));
    assert!(
        has_p,
        "p tag missing! tags={:?}",
        signed
            .tags
            .iter()
            .map(|t| t.as_slice().to_vec())
            .collect::<Vec<_>>()
    );
    assert_eq!(Kind1Codec.decode(&signed).unwrap(), ev);
}

#[test]
fn mention_vs_activity_disambiguation() {
    let keys = Keys::generate();
    // A note WITHOUT a p tag decodes as Activity.
    let act = DomainEvent::Activity(Activity {
        agent: agent(&keys, "coder"),
        project: "p".into(),
        text: "doing stuff".into(),
    });
    assert!(matches!(roundtrip(act, &keys), DomainEvent::Activity(_)));
}

#[test]
fn unrelated_kind_decodes_to_none() {
    let keys = Keys::generate();
    let reaction = EventBuilder::new(Kind::from(7u16), "+")
        .sign_with_keys(&keys)
        .unwrap();
    assert!(Kind1Codec.decode(&reaction).is_none());
}

mod groups;
mod legacy;
