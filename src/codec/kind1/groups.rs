use super::*;

// NIP-29 group management (operator/userNsec-signed) + relay-authored state.
pub const KIND_GROUP_CREATE: u16 = 9007;
pub const KIND_GROUP_PUT_USER: u16 = 9000;
pub const KIND_GROUP_REMOVE_USER: u16 = 9001;
pub const KIND_GROUP_EDIT_METADATA: u16 = 9002;
pub const KIND_GROUP_METADATA: u16 = 39000;
pub const KIND_GROUP_ADMINS: u16 = 39001;
pub const KIND_GROUP_MEMBERS: u16 = 39002;

// ── NIP-29 group management builders (signed by the operator's userNsec) ──────
//
// These sit outside the DomainEvent flow: they manage the relay's group, they
// aren't fabric domain events. The relay rules these encode were validated by
// `tests/nip29_probe.rs` against nip29.f7z.io. Recipe for an owned closed group:
//   group_create -> group_lock_closed -> group_put_user (per agent).

/// kind:9007 create-group with a client-chosen id (`h` == project slug). The
/// signer becomes the group admin. NOTE: a fresh group is OPEN until locked.
pub fn group_create(project: &str) -> Result<EventBuilder> {
    Ok(EventBuilder::new(kind(KIND_GROUP_CREATE), "").tags([project_tag(project)?]))
}

/// kind:9002 edit-metadata that locks the group `closed` (only members may write)
/// while keeping it `public` (anyone may read — required so the non-member daemon
/// connection still receives group events). Names the group after the slug.
pub fn group_lock_closed(project: &str) -> Result<EventBuilder> {
    Ok(EventBuilder::new(kind(KIND_GROUP_EDIT_METADATA), "").tags([
        project_tag(project)?,
        tag(&["name", project])?,
        tag(&["closed"])?,
        tag(&["public"])?,
    ]))
}

/// kind:9000 put-user adding `pubkey` to the group as a member, so it can publish
/// presence/activity/mentions into the now-closed group.
pub fn group_put_user(project: &str, pubkey: &str) -> Result<EventBuilder> {
    Ok(EventBuilder::new(kind(KIND_GROUP_PUT_USER), "")
        .tags([project_tag(project)?, tag(&["p", pubkey, "member"])?]))
}

/// kind:9001 remove-user removing `pubkey` from the group.
pub fn group_remove_user(project: &str, pubkey: &str) -> Result<EventBuilder> {
    Ok(EventBuilder::new(kind(KIND_GROUP_REMOVE_USER), "")
        .tags([project_tag(project)?, tag(&["p", pubkey])?]))
}
