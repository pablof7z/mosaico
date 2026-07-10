use super::*;

#[test]
fn profile_roundtrip() {
    let keys = Keys::generate();
    let ev = DomainEvent::Profile(crate::domain::Profile {
        agent: agent(&keys, "developer/coder"),
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
    assert_eq!(signed.content, r#"{"name":"developer/coder"}"#);
    assert!(has_tag(&signed, "agent-slug", "developer"));
}

#[test]
fn profile_decode_strips_backend_suffix_for_routing_slug() {
    let keys = Keys::generate();
    let event = EventBuilder::new(
        Kind::from(KIND_PROFILE),
        r#"{"name":"developer1@remoteBackend"}"#,
    )
    .tags([
        tag(&["host", "remoteBackend"]).unwrap(),
        tag(&["agent-slug", "developer"]).unwrap(),
    ])
    .sign_with_keys(&keys)
    .unwrap();

    match Nip29WireCodec.decode_event(&event) {
        Some(DomainEvent::Profile(p)) => {
            assert_eq!(p.agent.slug, "developer/developer1");
            assert_eq!(p.agent_slug, "developer");
            assert_eq!(p.host, "remoteBackend");
        }
        other => panic!("expected profile, got {other:?}"),
    }
}
