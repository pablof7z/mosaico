use super::super::*;
use super::reg;

#[test]
fn identities_bind_and_resolve() {
    let s = Store::open_memory().unwrap();
    let sid = s.register_session(&reg("claude-code", "x", "h1")).unwrap();
    s.upsert_identity(&Identity {
        pubkey: "derived".into(),
        agent_slug: "agent".into(),
        codename: "willow-echo-042".into(),
        session_id: String::new(),
        channel_h: "h1".into(),
        native_id: String::new(),
        alive: false,
        created_at: 1,
    })
    .unwrap();
    s.bind_session_identity("derived", &sid, "native-1", true)
        .unwrap();
    let r = s.identity_for_session(&sid).unwrap().unwrap();
    assert_eq!(r.pubkey, "derived");
    assert_eq!(r.codename, "willow-echo-042");
    assert_eq!(r.native_id, "native-1");
    assert!(r.alive);
    let by_channel = s
        .get_identity_for_channel("derived", "h1")
        .unwrap()
        .unwrap();
    assert_eq!(by_channel.session_id, sid);
    assert_eq!(
        s.list_identity_pubkeys().unwrap(),
        vec!["derived".to_string()]
    );
}

#[test]
fn project_roots_roundtrip() {
    let s = Store::open_memory().unwrap();
    s.upsert_project_root("h1", "/abs/path", 1).unwrap();
    assert_eq!(s.project_root("h1").unwrap().unwrap(), "/abs/path");
    s.upsert_project_root("h1", "/abs/other", 2).unwrap();
    assert_eq!(s.project_root("h1").unwrap().unwrap(), "/abs/other");
}
