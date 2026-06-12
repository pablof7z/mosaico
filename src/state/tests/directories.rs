use super::*;

#[test]
fn pending_agents_lifecycle() {
    let s = Store::open_memory().unwrap();
    s.upsert_pending_agent("pkX", "intruder", "their-box", "owner1", 5)
        .unwrap();
    s.upsert_pending_agent("pkX", "intruder", "their-box", "owner1", 6)
        .unwrap(); // upsert
    let pend = s.list_pending_agents().unwrap();
    assert_eq!(pend.len(), 1);
    assert_eq!(pend[0].slug, "intruder");
    s.remove_pending_agent("pkX").unwrap();
    assert!(s.list_pending_agents().unwrap().is_empty());
}

#[test]
fn session_prefix_lookup() {
    let s = Store::open_memory().unwrap();
    s.upsert_peer_session("abcdef123456", "pk", "coder", "proj", "host", "", 1)
        .unwrap();
    let found = s.find_peer_session_by_prefix("abcdef").unwrap().unwrap();
    assert_eq!(found.pubkey, "pk");
    assert!(s.find_peer_session_by_prefix("zzzz").unwrap().is_none());
}

#[test]
fn turn_delta_peer_sessions_can_be_project_scoped() {
    let s = Store::open_memory().unwrap();
    s.upsert_peer_session("sess-a", "pk-a", "same", "current", "host", "", 100)
        .unwrap();
    s.upsert_peer_session("sess-b", "pk-b", "other", "elsewhere", "host", "", 100)
        .unwrap();

    let scoped = s.list_new_peer_sessions(50, 50, Some("current")).unwrap();
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].session_id, "sess-a");

    let all = s.list_new_peer_sessions(50, 50, None).unwrap();
    assert_eq!(all.len(), 2);
}

#[test]
fn turn_delta_status_changes_can_be_project_scoped() {
    let s = Store::open_memory().unwrap();
    s.upsert_profile("pk-a", "alpha", "host", 1).unwrap();
    s.upsert_profile("pk-b", "bravo", "host", 1).unwrap();
    s.set_agent_status("pk-a", "current", Some("sess-a"), "working here", 100)
        .unwrap();
    s.set_agent_status("pk-b", "elsewhere", Some("sess-b"), "working there", 100)
        .unwrap();

    let scoped = s.list_status_changes_since(50, Some("current")).unwrap();
    assert_eq!(
        scoped,
        vec![(
            "alpha".to_string(),
            "current".to_string(),
            "working here".to_string()
        )]
    );

    let all = s.list_status_changes_since(50, None).unwrap();
    assert_eq!(all.len(), 2);
}

#[test]
fn owned_groups_roundtrip_and_idempotent() {
    let s = Store::open_memory().unwrap();
    assert!(!s.is_group_owned("proj").unwrap());
    s.mark_group_owned("proj", 100).unwrap();
    assert!(s.is_group_owned("proj").unwrap());
    // Re-marking is a no-op (keeps the original created_at), not an error.
    s.mark_group_owned("proj", 200).unwrap();
    assert!(s.is_group_owned("proj").unwrap());
    assert!(!s.is_group_owned("other").unwrap());
}

#[test]
fn group_member_upsert_and_query() {
    let s = Store::open_memory().unwrap();
    assert!(!s.is_group_member("proj", "pk-a").unwrap());
    s.upsert_group_member("proj", "pk-a", "member", 100)
        .unwrap();
    assert!(s.is_group_member("proj", "pk-a").unwrap());
    // Membership is per (project, pubkey).
    assert!(!s.is_group_member("other", "pk-a").unwrap());
    assert!(!s.is_group_member("proj", "pk-b").unwrap());
    // Upsert is idempotent on the primary key.
    s.upsert_group_member("proj", "pk-a", "admin", 200).unwrap();
    assert!(s.is_group_member("proj", "pk-a").unwrap());
}

#[test]
fn replace_group_members_is_authoritative() {
    let s = Store::open_memory().unwrap();
    s.upsert_group_member("proj", "stale", "member", 100)
        .unwrap();
    // A relay 39002 snapshot replaces the whole set: 'stale' drops out.
    s.replace_group_members(
        "proj",
        &[
            ("pk-a".into(), "member".into()),
            ("pk-b".into(), "admin".into()),
        ],
        300,
    )
    .unwrap();
    assert!(!s.is_group_member("proj", "stale").unwrap());
    assert!(s.is_group_member("proj", "pk-a").unwrap());
    assert!(s.is_group_member("proj", "pk-b").unwrap());
    // Scoped to the project — a different group is untouched.
    s.upsert_group_member("other", "pk-x", "member", 100)
        .unwrap();
    s.replace_group_members("proj", &[], 400).unwrap();
    assert!(!s.is_group_member("proj", "pk-a").unwrap());
    assert!(s.is_group_member("other", "pk-x").unwrap());
}
