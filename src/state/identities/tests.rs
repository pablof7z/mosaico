use super::*;

fn reg(ext: &str, channel: &str, pubkey: &str) -> RegisterSession {
    RegisterSession {
        harness: "codex".into(),
        external_id_kind: "harness_session".into(),
        external_id: ext.into(),
        agent_pubkey: pubkey.into(),
        agent_slug: "smith".into(),
        channel_h: channel.into(),
        child_pid: Some(42),
        transcript_path: None,
        resume_id: format!("resume-{ext}"),
        now: 1000,
    }
}

#[test]
fn identity_for_session_binds_each_session_to_its_own_pubkey() {
    let s = Store::open_memory().unwrap();
    let sid0 = s.register_session(&reg("s0", "h-a", "pk0")).unwrap();
    let sid1 = s.register_session(&reg("s1", "h-b", "pk1")).unwrap();

    s.upsert_identity(&Identity {
        pubkey: "pk0".into(),
        agent_slug: "smith".into(),
        codename: "willow-echo-042".into(),
        session_id: sid0.clone(),
        channel_h: "h-a".into(),
        native_id: "native-0".into(),
        alive: true,
        created_at: 1,
    })
    .unwrap();
    s.upsert_identity(&Identity {
        pubkey: "pk1".into(),
        agent_slug: "smith".into(),
        codename: "cedar-orbit-113".into(),
        session_id: sid1.clone(),
        channel_h: "h-b".into(),
        native_id: "native-1".into(),
        alive: true,
        created_at: 2,
    })
    .unwrap();

    let first = s.identity_for_session(&sid0).unwrap().unwrap();
    let second = s.identity_for_session(&sid1).unwrap().unwrap();

    assert_eq!(first.pubkey, "pk0");
    assert_eq!(first.codename, "willow-echo-042");
    assert_eq!(first.channel_h, "h-a");
    assert_eq!(first.native_id, "native-0");
    assert_eq!(second.pubkey, "pk1");
    assert_eq!(second.codename, "cedar-orbit-113");
    assert_eq!(second.channel_h, "h-b");
    assert_eq!(second.native_id, "native-1");
}
