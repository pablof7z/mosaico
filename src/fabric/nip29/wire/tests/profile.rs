use super::*;

#[test]
fn profile_roundtrip() {
    let keys = Keys::generate();
    let ev = DomainEvent::Profile(crate::domain::Profile {
        agent: agent(&keys, "willow-echo-042-developer"),
        agent_slug: "developer".into(),
        host: "pablos' laptop".into(),
        workspace: "29er-next".into(),
        owners: vec!["09d4".repeat(16)],
        is_backend: false,
        agents: Vec::new(),
    });
    assert_eq!(roundtrip(ev.clone(), &keys), ev);

    let signed = Nip29WireCodec
        .encode_event(&ev)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    assert_eq!(signed.content, r#"{"name":"willow-echo-042-developer"}"#);
    assert!(has_tag(&signed, "agent-slug", "developer"));
    assert!(has_tag(&signed, "workspace", "29er-next"));
}

#[test]
fn backend_profile_advertises_managed_agents_as_tags() {
    let keys = Keys::generate();
    let profile = crate::domain::Profile::backend_named(
        keys.public_key().to_hex(),
        "laptop (tenex-edge)",
        "laptop",
        Vec::new(),
    )
    .with_agents(vec![
        ("developer".into(), "writes and reviews code".into()),
        ("writer".into(), String::new()),
    ]);
    let ev = DomainEvent::Profile(profile);

    // Round-trips: `["agent", slug, desc]` tags survive encode -> decode, and an
    // empty description decodes back to empty (not dropped).
    assert_eq!(roundtrip(ev.clone(), &keys), ev);

    let signed = Nip29WireCodec
        .encode_event(&ev)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    let agent_tags: Vec<Vec<String>> = signed
        .tags
        .iter()
        .map(|t| t.as_slice().to_vec())
        .filter(|s| s.first().map(String::as_str) == Some("agent"))
        .collect();
    assert_eq!(
        agent_tags,
        vec![
            vec![
                "agent".to_string(),
                "developer".to_string(),
                "writes and reviews code".to_string()
            ],
            vec!["agent".to_string(), "writer".to_string(), String::new()],
        ]
    );
}

#[test]
fn non_backend_profile_omits_agent_tags() {
    let keys = Keys::generate();
    // Agent (non-backend) profiles never advertise a managed set.
    let mut profile = crate::domain::Profile::agent(
        agent(&keys, "willow-echo-042-developer"),
        "developer",
        "laptop",
        Vec::new(),
    );
    profile.agents = vec![("leaked".into(), "should not be encoded".into())];
    let signed = Nip29WireCodec
        .encode_event(&DomainEvent::Profile(profile))
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    assert!(
        !signed
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(String::as_str) == Some("agent")),
        "non-backend profile must not emit agent tags"
    );
}

#[test]
fn retired_profile_roundtrip_keeps_npub_as_the_name() {
    let keys = Keys::generate();
    let npub = keys.public_key().to_bech32().unwrap();
    let profile = DomainEvent::Profile(crate::domain::Profile {
        agent: crate::domain::AgentRef::new(keys.public_key().to_hex(), npub.clone()),
        agent_slug: "developer".into(),
        host: "remoteBackend".into(),
        workspace: String::new(),
        owners: Vec::new(),
        is_backend: false,
        agents: Vec::new(),
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
            assert_eq!(p.workspace, "");
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
