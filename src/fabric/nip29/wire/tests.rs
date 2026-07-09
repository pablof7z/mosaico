use super::*;

mod profile;

pub(super) fn roundtrip(ev: DomainEvent, keys: &Keys) -> DomainEvent {
    let codec = Nip29WireCodec;
    let builder = codec.encode_event(&ev).expect("encode");
    let signed = builder.sign_with_keys(keys).expect("sign");
    codec.decode_event(&signed).expect("decode")
}

pub(super) fn agent(keys: &Keys, slug: &str) -> AgentRef {
    AgentRef::new(keys.public_key().to_hex(), slug)
}

pub(super) fn has_tag(event: &Event, name: &str, value: &str) -> bool {
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

fn status(keys: &Keys, busy: bool, rel_cwd: &str) -> DomainEvent {
    DomainEvent::Status(Status {
        // Empty slug keeps the default status roundtrip focused on required
        // fields; non-empty slug emission is covered separately.
        agent: AgentRef::new(keys.public_key().to_hex(), String::new()),
        channels: vec!["tenex-edge".into()],
        session_id: "sess-123".into(),
        host: "laptop".into(),
        title: "fixing the auth bug".into(),
        activity: if busy {
            "reading the diff".into()
        } else {
            String::new()
        },
        busy,
        rel_cwd: rel_cwd.into(),
        // Default helper builds a non-expiring status; the expiration
        // roundtrip is covered by `status_expiration_roundtrips_and_emits_tag`.
        expires_at: None,
    })
}

#[test]
fn status_roundtrip() {
    let keys = Keys::generate();
    let ev = status(&keys, true, "");
    assert_eq!(roundtrip(ev.clone(), &keys), ev);
}

#[test]
fn status_rel_cwd_roundtrips_and_emits_tag() {
    let keys = Keys::generate();
    let ev = status(&keys, true, "worktree1");
    // The relative dir survives encode→decode …
    assert_eq!(roundtrip(ev.clone(), &keys), ev);
    // … and lands as a `rel-cwd` tag on the wire.
    let signed = Nip29WireCodec
        .encode_event(&ev)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    assert!(has_tag(&signed, "rel-cwd", "worktree1"));
    // … and the wire event has NO agent tag.
    assert!(!has_tag_name(&signed, "agent"));
}

#[test]
fn empty_rel_cwd_emits_no_tag_and_decodes_empty() {
    let keys = Keys::generate();
    let ev = status(&keys, false, "");
    let signed = Nip29WireCodec
        .encode_event(&ev)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    assert!(!has_tag_name(&signed, "rel-cwd"));
    match Nip29WireCodec.decode_event(&signed) {
        Some(DomainEvent::Status(s)) => assert_eq!(s.rel_cwd, ""),
        other => panic!("expected status, got {other:?}"),
    }
}

#[test]
fn status_is_per_group_self_contained_signal() {
    // The unified shape: `d == session_id`, `h == group_id`, full tag set,
    // content = live activity, title persisted as a tag even when busy.
    let keys = Keys::generate();
    let signed = Nip29WireCodec
        .encode_event(&status(&keys, true, "worktree1"))
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    assert_eq!(signed.kind.as_u16(), KIND_STATUS);
    assert!(has_tag(&signed, "d", "sess-123"));
    assert!(has_tag(&signed, "h", "tenex-edge"));
    // The session id is the address, not a duplicate side tag.
    assert!(!has_tag_name(&signed, "session-id"));
    assert!(has_tag(&signed, "title", "fixing the auth bug"));
    assert!(has_tag(&signed, "status", "busy"));
    assert!(has_tag(&signed, "host", "laptop"));
    assert!(has_tag(&signed, "rel-cwd", "worktree1"));
    // A None `expires_at` publishes no NIP-40 expiration tag.
    assert!(!has_tag_name(&signed, "expiration"));
    // The live activity is the content, not a tag.
    assert_eq!(signed.content, "reading the diff");
    assert!(!has_tag_name(&signed, "activity"));
    // Empty slugs are omitted rather than emitting a useless hint.
    assert!(!has_tag_name(&signed, "slug"));
    // No legacy presence-heartbeat artifacts, no self-asserted agent tag.
    assert!(!has_tag(&signed, "d", "tenex-edge-presence:sess-123"));
    assert!(!has_tag_name(&signed, "agent"));
}

#[test]
fn status_slug_is_convenience_hint_not_agent_tag() {
    let keys = Keys::generate();
    let ev = DomainEvent::Status(Status {
        agent: agent(&keys, "coder"),
        channels: vec!["tenex-edge".into()],
        session_id: "sess-123".into(),
        host: "laptop".into(),
        title: "fixing the auth bug".into(),
        activity: "reading the diff".into(),
        busy: true,
        rel_cwd: String::new(),
        expires_at: None,
    });
    let signed = Nip29WireCodec
        .encode_event(&ev)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    assert!(has_tag(&signed, "slug", "coder"));
    assert!(!has_tag_name(&signed, "agent"));
    assert_eq!(roundtrip(ev.clone(), &keys), ev);
}

#[test]
fn idle_status_marks_idle_and_keeps_title() {
    let keys = Keys::generate();
    let signed = Nip29WireCodec
        .encode_event(&status(&keys, false, ""))
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    assert!(has_tag(&signed, "status", "idle"));
    // Title persists across idle; content (live activity) is empty.
    assert!(has_tag(&signed, "title", "fixing the auth bug"));
    assert_eq!(signed.content, "");
    match Nip29WireCodec.decode_event(&signed) {
        Some(DomainEvent::Status(s)) => {
            assert!(s.is_idle());
            assert_eq!(s.title, "fixing the auth bug");
            assert_eq!(s.activity, "");
        }
        other => panic!("expected status, got {other:?}"),
    }
}

#[test]
fn status_expiration_roundtrips_and_emits_tag() {
    // A Some(expires_at) rides the wire as a NIP-40 `["expiration", ts]` tag
    // and decodes back to the same value — liveness IS this event's freshness.
    let keys = Keys::generate();
    let mut ev = status(&keys, true, "");
    if let DomainEvent::Status(s) = &mut ev {
        s.expires_at = Some(1_900_000_000);
    }
    assert_eq!(roundtrip(ev.clone(), &keys), ev);
    let signed = Nip29WireCodec
        .encode_event(&ev)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    assert!(has_tag(&signed, "expiration", "1900000000"));
    match Nip29WireCodec.decode_event(&signed) {
        Some(DomainEvent::Status(s)) => assert_eq!(s.expires_at, Some(1_900_000_000)),
        other => panic!("expected status, got {other:?}"),
    }
}

#[test]
fn status_session_address_can_differ_from_channel_h() {
    // Status is addressed by session id and can be visible in one or more h-tagged
    // channels; the session address is independent from each channel tag.
    let keys = Keys::generate();
    let event = EventBuilder::new(Kind::from(KIND_STATUS), "")
        .tags([
            tag(&["h", "tenex-edge"]).unwrap(),
            tag(&["d", "tenex-edge:sess-xyz"]).unwrap(),
            tag(&["status", "idle"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    match Nip29WireCodec.decode_event(&event) {
        Some(DomainEvent::Status(s)) => {
            assert_eq!(s.session_id.as_str(), "tenex-edge:sess-xyz");
            assert_eq!(s.channels, vec!["tenex-edge"]);
        }
        other => panic!("expected status, got {other:?}"),
    }
}

#[test]
fn status_session_id_d_is_accepted() {
    // Canonical shape: `d` is the session id; `h` is the channel id.
    let keys = Keys::generate();
    let event = EventBuilder::new(Kind::from(KIND_STATUS), "working on tests")
        .tags([
            tag(&["h", "tenex-edge"]).unwrap(),
            tag(&["d", "tenex-edge"]).unwrap(),
            tag(&["status", "busy"]).unwrap(),
            tag(&["title", "codec refactor"]).unwrap(),
            tag(&["host", "laptop"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    match Nip29WireCodec.decode_event(&event) {
        Some(DomainEvent::Status(s)) => {
            assert_eq!(s.channels, vec!["tenex-edge"]);
            assert_eq!(s.activity, "working on tests");
            assert_eq!(s.title, "codec refactor");
            assert!(s.busy);
        }
        other => panic!("expected status, got {other:?}"),
    }
}

#[test]
fn activity_roundtrip() {
    // Slug is NOT on the wire; decoded activity always has empty slug.
    let keys = Keys::generate();
    let ev = DomainEvent::Activity(Activity {
        agent: AgentRef::new(keys.public_key().to_hex(), String::new()),
        channel: "tenex-edge".into(),
        text: "fixing the auth bug".into(),
    });
    assert_eq!(roundtrip(ev.clone(), &keys), ev);
}

#[test]
fn activity_uses_nip29_h_tag_not_hashtag() {
    let keys = Keys::generate();
    let ev = DomainEvent::Activity(Activity {
        agent: agent(&keys, "coder"),
        channel: "tenex-edge".into(),
        text: "fixing the auth bug".into(),
    });
    let signed = Nip29WireCodec
        .encode_event(&ev)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    assert!(has_tag(&signed, "h", "tenex-edge"));
    assert!(!has_tag_name(&signed, "t"));
}

#[test]
fn unrelated_kind_decodes_to_none() {
    let keys = Keys::generate();
    let reaction = EventBuilder::new(Kind::from(7u16), "+")
        .sign_with_keys(&keys)
        .unwrap();
    assert!(Nip29WireCodec.decode_event(&reaction).is_none());
}

#[test]
fn kind_24011_presence_is_ignored() {
    let keys = Keys::generate();
    let event = EventBuilder::new(Kind::from(24011u16), "")
        .tags([
            tag(&["h", "tenex-edge"]).unwrap(),
            tag(&["legacy-session", "sess-123"]).unwrap(),
            tag(&["expiration", "1900000000"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    assert!(Nip29WireCodec.decode_event(&event).is_none());
}

#[test]
fn t_only_channel_notes_are_ignored() {
    // A kind:1 with only a `t` tag (old hashtag shape, no `h` tag) → None
    // (no `h` tag means no channel, so channel_from_tags returns None).
    let keys = Keys::generate();
    let event = EventBuilder::new(Kind::from(1u16), "old shape")
        .tags([tag(&["t", "tenex-edge"]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    assert!(Nip29WireCodec.decode_event(&event).is_none());
}

#[test]
fn chat_message_encodes_as_kind9_with_group_and_pubkey_mention_only() {
    let keys = Keys::generate();
    let mentioned_pk = "dd".repeat(32);
    let ev = DomainEvent::ChatMessage(ChatMessage {
        from: agent(&keys, "codex"),
        channel: "mychannel".into(),
        body: "status: tests are green".into(),
        mentioned_pubkey: Some(mentioned_pk.clone()),
    });
    let codec = Nip29WireCodec;
    let builder = codec.encode_event(&ev).expect("encode");
    let signed = builder.sign_with_keys(&keys).expect("sign");

    assert_eq!(signed.kind.as_u16(), KIND_CHAT);
    assert!(has_tag(&signed, "h", "mychannel"));
    assert!(has_tag(&signed, "p", &mentioned_pk));

    match codec.decode_event(&signed) {
        Some(DomainEvent::ChatMessage(chat)) => {
            assert_eq!(chat.channel, "mychannel");
            assert_eq!(chat.body, "status: tests are green");
            assert_eq!(chat.mentioned_pubkey, Some(mentioned_pk));
        }
        other => panic!("expected ChatMessage, got {other:?}"),
    }
}
