//! Persistence-foundation tests: canonical session identity, NIP-01 replacement,
//! NIP-40 status liveness, and unique-pubkey-per-channel membership.

use super::*;

fn reg(harness: &str, ext: &str, channel: &str) -> RegisterSession {
    RegisterSession {
        harness: harness.into(),
        external_id_kind: "harness_session".into(),
        external_id: ext.into(),
        agent_pubkey: "pk-agent".into(),
        agent_slug: "agent".into(),
        channel_h: channel.into(),
        child_pid: Some(42),
        transcript_path: Some("/t/x.jsonl".into()),
        resume_id: String::new(),
        now: 1000,
    }
}

#[test]
fn canonical_id_stable_across_external_id_rotation() {
    let s = Store::open_memory().unwrap();
    let canonical = s.register_session(&reg("claude-code", "ext-A", "h1")).unwrap();
    // A rotated harness id repointed onto the same canonical session.
    s.put_alias("claude-code", "resume", "ext-B", &canonical, 1500)
        .unwrap();
    // Mutating by EITHER external id must resolve to the canonical row.
    s.set_working("ext-A", true, 2000).unwrap();
    assert!(s.get_session("ext-B").unwrap().unwrap().working);
    assert_eq!(
        s.get_session("ext-A").unwrap().unwrap().session_id,
        canonical
    );
}

#[test]
fn register_is_idempotent_per_external_id() {
    let s = Store::open_memory().unwrap();
    let a = s.register_session(&reg("codex", "x1", "h1")).unwrap();
    let b = s.register_session(&reg("codex", "x1", "h1")).unwrap();
    assert_eq!(a, b);
    assert_eq!(s.list_alive_sessions().unwrap().len(), 1);
}

#[test]
fn mark_dead_resolves_external_id() {
    let s = Store::open_memory().unwrap();
    s.register_session(&reg("opencode", "o1", "h1")).unwrap();
    s.mark_dead("o1").unwrap();
    assert!(!s.get_session("o1").unwrap().unwrap().alive);
    assert!(s.list_alive_sessions().unwrap().is_empty());
}

#[test]
fn nip01_replaceable_replaces_by_kind_pubkey() {
    let s = Store::open_memory().unwrap();
    let mut ev = RelayEvent {
        id: "e1".into(),
        kind: 10002,
        pubkey: "pk".into(),
        created_at: 100,
        channel_h: String::new(),
        d_tag: String::new(),
        content: "old".into(),
        tags_json: "[]".into(),
    };
    assert!(s.insert_event(&ev).unwrap());
    ev.id = "e2".into();
    ev.created_at = 200;
    ev.content = "new".into();
    assert!(s.insert_event(&ev).unwrap());
    assert!(s.get_event("e1").unwrap().is_none());
    assert_eq!(s.get_event("e2").unwrap().unwrap().content, "new");
    // An older event loses the race and is not stored.
    ev.id = "e0".into();
    ev.created_at = 50;
    assert!(!s.insert_event(&ev).unwrap());
    assert!(s.get_event("e0").unwrap().is_none());
}

#[test]
fn nip01_addressable_replaces_by_kind_pubkey_dtag() {
    let s = Store::open_memory().unwrap();
    let mk = |id: &str, ts: u64, d: &str| RelayEvent {
        id: id.into(),
        kind: 30078,
        pubkey: "pk".into(),
        created_at: ts,
        channel_h: String::new(),
        d_tag: d.into(),
        content: String::new(),
        tags_json: "[]".into(),
    };
    assert!(s.insert_event(&mk("a", 1, "d1")).unwrap());
    assert!(s.insert_event(&mk("b", 1, "d2")).unwrap());
    // Replace d1 only; d2 survives (different coordinate).
    assert!(s.insert_event(&mk("c", 2, "d1")).unwrap());
    assert!(s.get_event("a").unwrap().is_none());
    assert!(s.get_event("b").unwrap().is_some());
    assert!(s.get_event("c").unwrap().is_some());
}

#[test]
fn nip01_regular_appends() {
    let s = Store::open_memory().unwrap();
    let mk = |id: &str| RelayEvent {
        id: id.into(),
        kind: 1,
        pubkey: "pk".into(),
        created_at: 1,
        channel_h: "h1".into(),
        d_tag: String::new(),
        content: String::new(),
        tags_json: "[]".into(),
    };
    assert!(s.insert_event(&mk("n1")).unwrap());
    assert!(s.insert_event(&mk("n2")).unwrap());
    assert_eq!(s.chat_for_channel("h1", 0, 10).unwrap().len(), 2);
}

#[test]
fn nip40_expired_status_not_live() {
    let s = Store::open_memory().unwrap();
    let live = Status {
        pubkey: "pk1".into(),
        channel_h: "h1".into(),
        slug: "a".into(),
        title: "t".into(),
        activity: "act".into(),
        busy: true,
        last_seen: 100,
        updated_at: 100,
        expiration: 200,
    };
    let expired = Status {
        pubkey: "pk2".into(),
        expiration: 50,
        ..live.clone()
    };
    s.upsert_status(&live).unwrap();
    s.upsert_status(&expired).unwrap();
    let now = 150;
    let rows = s.live_status_for_channel("h1", now).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].pubkey, "pk1");
}

#[test]
fn pubkey_unique_per_channel_admin_supersedes_member() {
    let s = Store::open_memory().unwrap();
    s.replace_channel_members("h1", &["pk1".into(), "pk2".into()], 10)
        .unwrap();
    s.replace_channel_admins("h1", &["pk1".into()], 20).unwrap();
    assert!(s.is_channel_admin("h1", "pk1").unwrap());
    assert!(!s.is_channel_admin("h1", "pk2").unwrap());
    assert!(s.is_channel_member("h1", "pk2").unwrap());
    // pk1 appears once, as admin.
    assert_eq!(s.count_channel_members("h1").unwrap(), 2);
    assert_eq!(
        s.list_channels_where_admin("pk1").unwrap(),
        vec!["h1".to_string()]
    );
}

#[test]
fn inbox_idempotency_and_delivery() {
    let s = Store::open_memory().unwrap();
    let sid = s.register_session(&reg("claude-code", "x", "h1")).unwrap();
    assert!(s
        .enqueue_inbox("ev1", &sid, "from", "h1", "hi", 100)
        .unwrap());
    // Duplicate is ignored (idempotent).
    assert!(!s
        .enqueue_inbox("ev1", &sid, "from", "h1", "hi", 100)
        .unwrap());
    assert!(s.is_event_handled("ev1", "x").unwrap());
    assert_eq!(s.drain_pending_for_session("x").unwrap().len(), 1);
    s.mark_delivered("ev1", "x", 200).unwrap();
    assert!(s.drain_pending_for_session("x").unwrap().is_empty());
}

#[test]
fn outbox_publish_and_retry() {
    let s = Store::open_memory().unwrap();
    let id = s.enqueue_outbox("{\"k\":1}", 100).unwrap();
    assert_eq!(s.drain_outbox(10).unwrap().len(), 1);
    s.mark_failed(id, "relay down").unwrap();
    let pending = s.drain_outbox(10).unwrap();
    assert_eq!(pending[0].retries, 1);
    s.mark_published(id).unwrap();
    assert!(s.drain_outbox(10).unwrap().is_empty());
}

#[test]
fn channels_root_vs_subchannel() {
    let s = Store::open_memory().unwrap();
    s.upsert_channel("proj", "P", "", "", 1).unwrap();
    s.upsert_channel("task", "T", "", "proj", 1).unwrap();
    assert!(s.is_root_channel("proj").unwrap());
    assert!(!s.is_root_channel("task").unwrap());
    assert_eq!(s.channel_parent("task").unwrap().unwrap(), "proj");
}

#[test]
fn channel_id_for_name_resolves_within_parent() {
    let s = Store::open_memory().unwrap();
    // Opaque id, human name "support" under project "proj".
    s.upsert_channel("ab12cd34", "support", "", "proj", 10).unwrap();
    assert_eq!(
        s.channel_id_for_name("proj", "support").unwrap().as_deref(),
        Some("ab12cd34")
    );
    // Unknown name → None.
    assert_eq!(s.channel_id_for_name("proj", "nope").unwrap(), None);
    // Same name under a DIFFERENT parent is a distinct channel (allowed).
    s.upsert_channel("ff99ff99", "support", "", "other", 10).unwrap();
    assert_eq!(
        s.channel_id_for_name("other", "support").unwrap().as_deref(),
        Some("ff99ff99")
    );
    // Legacy duplicate (parent, name): most-recently-updated wins.
    s.upsert_channel("zz000000", "support", "", "proj", 20).unwrap();
    assert_eq!(
        s.channel_id_for_name("proj", "support").unwrap().as_deref(),
        Some("zz000000")
    );
}

#[test]
fn identities_bind_and_resolve() {
    let s = Store::open_memory().unwrap();
    let sid = s.register_session(&reg("claude-code", "x", "h1")).unwrap();
    s.upsert_identity(&Identity {
        pubkey: "derived".into(),
        base_pubkey: "base".into(),
        agent_slug: "agent".into(),
        ordinal: 1,
        session_id: String::new(),
        channel_h: "h1".into(),
        native_id: String::new(),
        alive: false,
        created_at: 1,
    })
    .unwrap();
    s.bind_session_identity("derived", &sid, "native-1", true)
        .unwrap();
    let r = s.resolve_identity_for_channel("base", "h1").unwrap().unwrap();
    assert_eq!(r.session_id, sid);
    assert_eq!(r.native_id, "native-1");
    assert!(r.alive);
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
