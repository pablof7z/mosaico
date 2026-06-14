use super::*;

#[test]
fn filters_cover_all_kinds_and_mentions() {
    let me = Keys::generate().public_key().to_hex();
    let scope = SubScope {
        authors: vec![Keys::generate().public_key().to_hex()],
        project: Some("tenex-edge".into()),
        mentions_to: Some(me),
        owners: vec![Keys::generate().public_key().to_hex()],
    };
    let filters = Kind1Codec.filters(&scope);
    // profiles, presence/status, notes, mentions-to-me, and NIP-29 group-state
    // (39000/39001/39002 by #d).
    assert_eq!(filters.len(), 5);
    let json = serde_json::to_string(&filters).unwrap();
    assert!(json.contains("\"#h\""));
    assert!(!json.contains("\"#t\""));
    // group-state filter present: addressable kinds scoped by #d=slug.
    assert!(json.contains("\"#d\""));
    assert!(json.contains("39002"));
}

#[test]
fn group_create_has_h_tag() {
    let b = group_create("tenex-edge").unwrap();
    let ev = b.sign_with_keys(&Keys::generate()).unwrap();
    assert_eq!(ev.kind.as_u16(), KIND_GROUP_CREATE);
    assert!(has_tag(&ev, "h", "tenex-edge"));
}

#[test]
fn group_lock_closed_is_closed_and_public() {
    let b = group_lock_closed("tenex-edge").unwrap();
    let ev = b.sign_with_keys(&Keys::generate()).unwrap();
    assert_eq!(ev.kind.as_u16(), KIND_GROUP_EDIT_METADATA);
    assert!(has_tag(&ev, "h", "tenex-edge"));
    assert!(has_tag(&ev, "name", "tenex-edge"));
    assert!(has_tag_name(&ev, "closed"));
    assert!(has_tag_name(&ev, "public"));
    // Must NOT be private — would blind the non-member daemon connection.
    assert!(!has_tag_name(&ev, "private"));
}

#[test]
fn group_put_user_tags_member() {
    let member = Keys::generate().public_key().to_hex();
    let b = group_put_user("tenex-edge", &member).unwrap();
    let ev = b.sign_with_keys(&Keys::generate()).unwrap();
    assert_eq!(ev.kind.as_u16(), KIND_GROUP_PUT_USER);
    assert!(has_tag(&ev, "h", "tenex-edge"));
    // p tag carries the member pubkey with the "member" role.
    assert!(ev.tags.iter().any(|t| {
        let s = t.as_slice();
        s.first().map(String::as_str) == Some("p")
            && s.get(1).map(String::as_str) == Some(member.as_str())
            && s.get(2).map(String::as_str) == Some("member")
    }));
}
