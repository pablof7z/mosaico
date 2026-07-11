use super::*;

#[test]
fn profile_roundtrip() {
    let keys = Keys::generate();
    let ev = DomainEvent::Profile(crate::domain::Profile {
        agent: agent(&keys, "willow-echo-042-developer"),
        agent_slug: "developer".into(),
        host: "pablos' laptop".into(),
        owners: vec!["09d4".repeat(16)],
        is_backend: false,
    });
    assert_eq!(roundtrip(ev.clone(), &keys), ev);

    let signed = Nip29WireCodec
        .encode_event(&ev)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    assert_eq!(signed.content, r#"{"name":"willow-echo-042-developer"}"#);
    assert!(has_tag(&signed, "agent-slug", "developer"));
}

#[test]
fn retired_profile_roundtrip_keeps_npub_as_the_name() {
    let keys = Keys::generate();
    let npub = keys.public_key().to_bech32().unwrap();
    let profile = DomainEvent::Profile(crate::domain::Profile {
        agent: crate::domain::AgentRef::new(keys.public_key().to_hex(), npub.clone()),
        agent_slug: "developer".into(),
        host: "remoteBackend".into(),
        owners: Vec::new(),
        is_backend: false,
    });
    let signed = Nip29WireCodec
        .encode_event(&profile)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    assert_eq!(
        signed.content,
        serde_json::json!({ "name": npub }).to_string()
    );
    assert_eq!(roundtrip(profile.clone(), &keys), profile);
}

#[test]
fn profile_decode_builds_handle_from_session_code_and_canonical_tag() {
    let keys = Keys::generate();
    let event = EventBuilder::new(Kind::from(KIND_PROFILE), r#"{"name":"willow-echo-042"}"#)
        .tags([
            tag(&["host", "remoteBackend"]).unwrap(),
            tag(&["agent-slug", "developer"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();

    match Nip29WireCodec.decode_event(&event) {
        Some(DomainEvent::Profile(p)) => {
            assert_eq!(p.agent.slug, "willow-echo-042-developer");
            assert_eq!(p.agent_slug, "developer");
            assert_eq!(p.host, "remoteBackend");
        }
        other => panic!("expected profile, got {other:?}"),
    }
}

#[test]
fn profile_decode_ignores_removed_camel_case_agent_slug_tag() {
    let keys = Keys::generate();
    let event = EventBuilder::new(Kind::from(KIND_PROFILE), r#"{"name":"willow-echo-042"}"#)
        .tags([
            tag(&["host", "remoteBackend"]).unwrap(),
            tag(&["agentSlug", "developer"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();

    match Nip29WireCodec.decode_event(&event) {
        Some(DomainEvent::Profile(p)) => {
            assert_eq!(p.agent.slug, "willow-echo-042");
            assert!(p.agent_slug.is_empty());
        }
        other => panic!("expected profile, got {other:?}"),
    }
}
