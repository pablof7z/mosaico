use super::*;

fn member(pubkey: &str, role: &str) -> crate::state::ChannelMember {
    crate::state::ChannelMember {
        channel_h: "chan".to_string(),
        pubkey: pubkey.to_string(),
        role: role.to_string(),
    }
}

#[test]
fn archive_removal_targets_keep_admins() {
    let targets = archive_removal_targets(&[
        member("admin-pk", "admin"),
        member("member-pk", "member"),
        member("duplicate-member-pk", "member"),
        member("duplicate-member-pk", "member"),
    ]);

    assert_eq!(
        targets,
        vec!["duplicate-member-pk".to_string(), "member-pk".to_string()]
    );
}
