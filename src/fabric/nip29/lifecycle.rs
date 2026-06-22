//! NIP-29 group lifecycle builders (operator-signed management events).
//!
//! These sit outside the DomainEvent flow: they manage the relay's group, they
//! aren't fabric domain events. The relay rules these encode were validated by
//! `tests/nip29_probe.rs` against nip29.f7z.io. Recipe for an owned closed group:
//!   group_create -> group_lock_closed -> group_put_user (per agent).

use crate::codec::kind1::{
    kind, KIND_GROUP_CREATE, KIND_GROUP_EDIT_METADATA, KIND_GROUP_PUT_USER, KIND_GROUP_REMOVE_USER,
};
use anyhow::Result;
use nostr_sdk::prelude::*;

fn tag(parts: &[&str]) -> Result<Tag> {
    Ok(Tag::parse(parts.iter().copied())?)
}

fn project_tag(project: &str) -> Result<Tag> {
    tag(&["h", project])
}

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

/// kind:9007 create-group for a CHILD (sub-)group, using `child_h` as the
/// client-chosen group id and declaring its `parent_h` relationship at creation.
/// The `["parent", parent_h]` tag rides on the 9007 itself: NIP-29 subgroup
/// relays (per nostr-protocol/nips#2319, e.g. croissant) validate the parent at
/// create time (parent must exist; signer must be a parent admin; no cycles) and
/// re-emit the tag on the relay-authored kind:39000. The signer becomes the
/// subgroup admin and, as with any fresh group, it is OPEN until locked.
pub fn group_create_subgroup(child_h: &str, parent_h: &str) -> Result<EventBuilder> {
    Ok(EventBuilder::new(kind(KIND_GROUP_CREATE), "")
        .tags([project_tag(child_h)?, tag(&["parent", parent_h])?]))
}

/// kind:9002 edit-metadata that locks a CHILD group `closed` (only members may
/// write) while keeping it `public` (anyone may read — required so the
/// non-member daemon connection still receives group events) AND declares its
/// NIP-29 subgroup parent via a `["parent", parent_h]` tag (per
/// nostr-protocol/nips#2319). Unlike [`group_lock_closed`], `name` is a
/// human-readable display name rather than the slug. Must stay `public`, never
/// `private`, or the non-member daemon connection goes blind to the subgroup.
pub fn group_lock_closed_with_parent(
    child_h: &str,
    name: &str,
    parent_h: &str,
) -> Result<EventBuilder> {
    Ok(EventBuilder::new(kind(KIND_GROUP_EDIT_METADATA), "").tags([
        project_tag(child_h)?,
        tag(&["name", name])?,
        tag(&["parent", parent_h])?,
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

/// kind:9000 put-user adding `pubkey` with the `admin` role, granting it admin
/// permissions over the group (the relay lists it in kind:39001 with role=admin).
/// Same wire shape as [`group_put_user`] but with the role label set to "admin".
/// relay29 advertises the `admin`/`moderator` roles it accepts via kind:39003;
/// "admin" is the role tenex-edge grants to every whitelisted human pubkey.
pub fn group_put_admin(project: &str, pubkey: &str) -> Result<EventBuilder> {
    Ok(EventBuilder::new(kind(KIND_GROUP_PUT_USER), "")
        .tags([project_tag(project)?, tag(&["p", pubkey, "admin"])?]))
}

/// kind:9002 edit-metadata: set the group's `about` text. The relay validates
/// admin rights and re-publishes kind:39000 signed by the relay key.
pub fn group_edit_metadata(project: &str, about: &str) -> Result<EventBuilder> {
    Ok(EventBuilder::new(kind(KIND_GROUP_EDIT_METADATA), "")
        .tags([tag(&["d", project])?, tag(&["about", about])?]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_tag(event: &Event, name: &str, value: &str) -> bool {
        event.tags.iter().any(|t| {
            let s = t.as_slice();
            s.first().map(String::as_str) == Some(name)
                && s.get(1).map(String::as_str) == Some(value)
        })
    }

    fn has_tag_name(event: &Event, name: &str) -> bool {
        event
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(String::as_str) == Some(name))
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
    fn group_create_subgroup_has_h_and_parent_tags() {
        let b = group_create_subgroup("subgroup-support-a1b2c3d4", "tenex-edge").unwrap();
        let ev = b.sign_with_keys(&Keys::generate()).unwrap();
        assert_eq!(ev.kind.as_u16(), KIND_GROUP_CREATE);
        assert!(has_tag(&ev, "h", "subgroup-support-a1b2c3d4"));
        // The parent relationship must ride on the 9007 create (NIP #2319 relays
        // validate + re-emit it on 39000 from the create event).
        assert!(has_tag(&ev, "parent", "tenex-edge"));
    }

    #[test]
    fn subgroup_lock_has_parent_name_closed_public() {
        let b = group_lock_closed_with_parent(
            "subgroup-support-a1b2c3d4",
            "subgroup support",
            "tenex-edge",
        )
        .unwrap();
        let ev = b.sign_with_keys(&Keys::generate()).unwrap();
        assert_eq!(ev.kind.as_u16(), KIND_GROUP_EDIT_METADATA);
        assert!(has_tag(&ev, "h", "subgroup-support-a1b2c3d4"));
        assert!(has_tag(&ev, "name", "subgroup support"));
        assert!(has_tag(&ev, "parent", "tenex-edge"));
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

    #[test]
    fn group_remove_user_tags_member_without_role() {
        let member = Keys::generate().public_key().to_hex();
        let b = group_remove_user("tenex-edge", &member).unwrap();
        let ev = b.sign_with_keys(&Keys::generate()).unwrap();
        assert_eq!(ev.kind.as_u16(), KIND_GROUP_REMOVE_USER);
        assert!(has_tag(&ev, "h", "tenex-edge"));
        assert!(ev.tags.iter().any(|t| {
            let s = t.as_slice();
            s.first().map(String::as_str) == Some("p")
                && s.get(1).map(String::as_str) == Some(member.as_str())
                && s.get(2).is_none()
        }));
    }

    #[test]
    fn group_put_admin_tags_admin_role() {
        let pk = Keys::generate().public_key().to_hex();
        let b = group_put_admin("tenex-edge", &pk).unwrap();
        let ev = b.sign_with_keys(&Keys::generate()).unwrap();
        assert_eq!(ev.kind.as_u16(), KIND_GROUP_PUT_USER);
        assert!(has_tag(&ev, "h", "tenex-edge"));
        // p tag carries the pubkey with the "admin" role (not "member").
        assert!(ev.tags.iter().any(|t| {
            let s = t.as_slice();
            s.first().map(String::as_str) == Some("p")
                && s.get(1).map(String::as_str) == Some(pk.as_str())
                && s.get(2).map(String::as_str) == Some("admin")
        }));
    }
}
