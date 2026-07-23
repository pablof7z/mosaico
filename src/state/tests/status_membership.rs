use super::super::*;

#[test]
fn nip40_expired_status_not_live() {
    let s = Store::open_memory().unwrap();
    let live = Status {
        pubkey: "pk1".into(),
        channel_h: "h1".into(),
        slug: "a".into(),
        title: "t".into(),
        activity: "act".into(),
        state: crate::session_state::SessionState::Working,
        state_since: 100,
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
