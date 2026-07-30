use super::*;

#[test]
fn canonical_state_tag_is_accepted() {
    let keys = Keys::generate();
    let event = EventBuilder::new(Kind::from(KIND_STATUS), "working on tests")
        .tags([
            tag(&["h", "mosaico"]).unwrap(),
            tag(&["d", "status"]).unwrap(),
            tag(&["state", "working"]).unwrap(),
            tag(&["state-since", "42"]).unwrap(),
            tag(&["title", "codec refactor"]).unwrap(),
            tag(&["host", "laptop"]).unwrap(),
            tag(&["workspace", "mosaico"]).unwrap(),
            tag(&["slug", "codex"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    match Nip29WireCodec.decode_event(&event) {
        Some(DomainEvent::Status(s)) => {
            assert_eq!(s.channels, vec!["mosaico"]);
            assert_eq!(s.activity, "working on tests");
            assert_eq!(s.title, "codec refactor");
            assert_eq!(s.state, SessionState::Working);
        }
        other => panic!("expected status, got {other:?}"),
    }
}
