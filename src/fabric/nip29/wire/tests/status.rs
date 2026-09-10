use super::*;

fn has_tag_name(event: &Event, name: &str) -> bool {
    event
        .tags
        .iter()
        .any(|tag| tag.as_slice().first().map(String::as_str) == Some(name))
}

fn status(keys: &Keys, busy: bool, rel_cwd: &str) -> DomainEvent {
    DomainEvent::Status(Status {
        agent: AgentRef::new(keys.public_key().to_hex(), "coder"),
        channels: vec!["mosaico".into()],
        host: "laptop".into(),
        workspace: "mosaico".into(),
        branch: "feat/context".into(),
        title: "fixing the auth bug".into(),
        activity: if busy {
            "reading the diff".into()
        } else {
            String::new()
        },
        state: if busy {
            crate::session_state::SessionState::Working
        } else {
            crate::session_state::SessionState::Idle
        },
        state_since: 42,
        rel_cwd: rel_cwd.into(),
        expires_at: None,
        dispatch_event: None,
    })
}

#[test]
fn status_roundtrip() {
    let keys = Keys::generate();
    let event = status(&keys, true, "");
    assert_eq!(roundtrip(event.clone(), &keys), event);
}

#[test]
fn one_status_is_signed_with_every_group_context() {
    let keys = Keys::generate();
    let mut event = status(&keys, true, "");
    let DomainEvent::Status(status) = &mut event else {
        unreachable!()
    };
    status.channels = vec!["alpha".into(), "beta".into()];

    let signed = signed_as_published(&event, &keys);
    let groups = signed
        .tags
        .iter()
        .filter_map(|tag| {
            let row = tag.as_slice();
            (row.first().map(String::as_str) == Some("h")).then(|| row[1].clone())
        })
        .collect::<Vec<_>>();

    assert_eq!(groups, ["alpha", "beta"]);
    assert_eq!(roundtrip(event.clone(), &keys), event);
}

#[test]
fn status_rel_cwd_roundtrips_and_emits_tag() {
    let keys = Keys::generate();
    let event = status(&keys, true, "worktree1");
    assert_eq!(roundtrip(event.clone(), &keys), event);
    let signed = Nip29WireCodec
        .encode_event(&event)
        .unwrap()
        .finalize(&keys)
        .unwrap();
    assert!(has_tag(&signed, "rel-cwd", "worktree1"));
    assert!(!has_tag_name(&signed, "agent"));
}

#[test]
fn empty_rel_cwd_emits_no_tag_and_decodes_empty() {
    let keys = Keys::generate();
    let event = status(&keys, false, "");
    let signed = Nip29WireCodec
        .encode_event(&event)
        .unwrap()
        .finalize(&keys)
        .unwrap();
    assert!(!has_tag_name(&signed, "rel-cwd"));
    match Nip29WireCodec.decode_event(&signed) {
        Some(DomainEvent::Status(status)) => assert_eq!(status.rel_cwd, ""),
        other => panic!("expected status, got {other:?}"),
    }
}

#[test]
fn status_is_per_group_self_contained_signal() {
    let keys = Keys::generate();
    let signed = Nip29WireCodec
        .encode_event(&status(&keys, true, "worktree1"))
        .unwrap()
        .finalize(&keys)
        .unwrap();
    assert_eq!(signed.kind.as_u16(), KIND_STATUS);
    assert!(has_tag(&signed, "d", "status"));
    assert!(!has_tag_name(&signed, "h"));
    assert!(!has_tag_name(&signed, "session-id"));
    assert!(has_tag(&signed, "title", "fixing the auth bug"));
    assert!(has_tag(&signed, "state", "working"));
    assert!(has_tag(&signed, "state-since", "42"));
    assert!(has_tag(&signed, "host", "laptop"));
    assert!(has_tag(&signed, "workspace", "mosaico"));
    assert!(has_tag(&signed, "branch", "feat/context"));
    assert!(has_tag(&signed, "rel-cwd", "worktree1"));
    assert!(!has_tag_name(&signed, "expiration"));
    assert_eq!(signed.content, "reading the diff");
    assert!(!has_tag_name(&signed, "activity"));
    assert!(has_tag(&signed, "slug", "coder"));
    assert!(!has_tag_name(&signed, "agent"));
}

#[test]
fn status_slug_is_canonical_hint_not_agent_tag() {
    let keys = Keys::generate();
    let event = DomainEvent::Status(Status {
        agent: agent(&keys, "coder"),
        channels: vec!["mosaico".into()],
        host: "laptop".into(),
        workspace: "mosaico".into(),
        branch: "feat/context".into(),
        title: "fixing the auth bug".into(),
        activity: "reading the diff".into(),
        state: crate::session_state::SessionState::Working,
        state_since: 42,
        rel_cwd: String::new(),
        expires_at: None,
        dispatch_event: None,
    });
    let signed = Nip29WireCodec
        .encode_event(&event)
        .unwrap()
        .finalize(&keys)
        .unwrap();
    assert!(has_tag(&signed, "slug", "coder"));
    assert!(!has_tag_name(&signed, "agent"));
    assert_eq!(roundtrip(event.clone(), &keys), event);
}

#[test]
fn idle_status_marks_idle_and_keeps_title() {
    let keys = Keys::generate();
    let signed = Nip29WireCodec
        .encode_event(&status(&keys, false, ""))
        .unwrap()
        .finalize(&keys)
        .unwrap();
    assert!(has_tag(&signed, "state", "idle"));
    assert!(has_tag(&signed, "title", "fixing the auth bug"));
    assert_eq!(signed.content, "");
    match Nip29WireCodec.decode_event(&signed) {
        Some(DomainEvent::Status(status)) => {
            assert!(status.is_idle());
            assert_eq!(status.title, "fixing the auth bug");
            assert_eq!(status.activity, "");
        }
        other => panic!("expected status, got {other:?}"),
    }
}

#[test]
fn status_expiration_roundtrips_and_emits_tag() {
    let keys = Keys::generate();
    let mut event = status(&keys, true, "");
    if let DomainEvent::Status(status) = &mut event {
        status.expires_at = Some(1_900_000_000);
    }
    assert_eq!(roundtrip(event.clone(), &keys), event);
    let signed = Nip29WireCodec
        .encode_event(&event)
        .unwrap()
        .finalize(&keys)
        .unwrap();
    assert!(has_tag(&signed, "expiration", "1900000000"));
    match Nip29WireCodec.decode_event(&signed) {
        Some(DomainEvent::Status(status)) => {
            assert_eq!(status.expires_at, Some(1_900_000_000))
        }
        other => panic!("expected status, got {other:?}"),
    }
}

#[test]
fn status_uses_constant_address_independent_from_channel_h() {
    let keys = Keys::generate();
    let event = EventBuilder::new(Kind::from(KIND_STATUS), "")
        .tags([
            tag(&["h", "mosaico"]).unwrap(),
            tag(&["d", "status"]).unwrap(),
            tag(&["state", "idle"]).unwrap(),
            tag(&["state-since", "42"]).unwrap(),
            tag(&["title", ""]).unwrap(),
            tag(&["host", "laptop"]).unwrap(),
            tag(&["workspace", "mosaico"]).unwrap(),
            tag(&["slug", "codex"]).unwrap(),
        ])
        .finalize(&keys)
        .unwrap();
    match Nip29WireCodec.decode_event(&event) {
        Some(DomainEvent::Status(status)) => assert_eq!(status.channels, vec!["mosaico"]),
        other => panic!("expected status, got {other:?}"),
    }
}

#[test]
fn status_private_runtime_id_d_is_rejected() {
    let keys = Keys::generate();
    let event = EventBuilder::new(Kind::from(KIND_STATUS), "working on tests")
        .tags([
            tag(&["h", "mosaico"]).unwrap(),
            tag(&["d", "mosaico-private-run"]).unwrap(),
            tag(&["status", "busy"]).unwrap(),
            tag(&["title", "codec refactor"]).unwrap(),
            tag(&["host", "laptop"]).unwrap(),
        ])
        .finalize(&keys)
        .unwrap();
    assert!(Nip29WireCodec.decode_event(&event).is_none());
}
