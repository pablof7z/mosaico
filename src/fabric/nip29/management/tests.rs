use nostr::prelude::{Event, FinalizeEvent, Keys};

use super::*;

fn rows(event: &Event) -> Vec<Vec<String>> {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect()
}

#[test]
fn every_management_builder_has_group_context_before_signing() {
    let keys = Keys::generate();
    let user = Keys::generate().public_key();
    let cases = [
        create_group("group", None).unwrap(),
        remove_users("group", &[user]).unwrap(),
        edit_metadata(
            "group",
            &MetadataEdit {
                name: Some("Group".into()),
                closed: true,
                ..Default::default()
            },
        )
        .unwrap(),
        create_group("group", Some("parent")).unwrap(),
        delete_group("group").unwrap(),
    ];
    let kinds = [9007, 9001, 9002, 9007, 9008];
    for (builder, expected_kind) in cases.into_iter().zip(kinds) {
        let event = builder.finalize(&keys).unwrap();
        assert_eq!(event.kind.as_u16(), expected_kind);
        assert!(rows(&event).contains(&vec!["h".into(), "group".into()]));
    }

    let put = put_users(
        "group",
        &[GroupUser {
            pubkey: user,
            roles: vec!["admin".into()],
        }],
    )
    .unwrap()
    .finalize(&keys)
    .unwrap();
    assert_eq!(put.kind.as_u16(), 9000);
    assert!(rows(&put).contains(&vec!["p".into(), user.to_hex(), "admin".into()]));
}

#[test]
fn empty_group_is_rejected() {
    assert!(create_group("", None).is_err());
}
