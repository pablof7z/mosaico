//! Mosaico's group policy, stated for NIP-29's own schemas to compose.
//!
//! NIP-29's schemas belong to `nmp_nip29::operations` and are called there
//! directly. **Nothing in this module composes a wire row.** It states what
//! Mosaico wants — the avatar URL, the visibility a workspace has — and hands
//! it to NMP's spelling; the rows, including the `h` group context, are NMP's
//! to mint.
//!
//! Every draft that once lived here is gone. Visibility (`open`/`closed`,
//! `public`/`private`) and `picture` went to [`nmp_nip29::GroupMetadataEdit`]
//! (NMP #1282); the subgroup `parent` row went to
//! [`nmp_nip29::create_group`] (NMP #1301), which is the only kind that
//! carries it — see [`group_lock_closed`] for what that means for kind:9002.

use nostr::EventBuilder;

/// Mosaico's own avatar policy. The `picture` ROW is NIP-29's and is composed
/// by `nmp_nip29::edit_metadata`; which URL goes in it is the product's.
fn picture_url(seed: &str) -> String {
    format!("https://api.dicebear.com/10.x/stripes/svg?seed={seed}")
}

/// A Mosaico workspace is readable by anyone and joinable only by invitation.
/// Stated once, composed by NIP-29's own kind:9002 spelling.
fn workspace_visibility(name: &str, picture_seed: &str) -> nmp_nip29::GroupMetadataEdit {
    nmp_nip29::GroupMetadataEdit {
        name: Some(name.to_string()),
        picture: Some(picture_url(picture_seed)),
        read_access: Some(nmp_nip29::ReadAccess::Public),
        join_access: Some(nmp_nip29::JoinAccess::Closed),
        ..nmp_nip29::GroupMetadataEdit::default()
    }
}

/// NMP's builder as `nostr`'s, for the signing doors that still take one.
///
/// `allow_self_tagging` is required rather than defensive: `nostr`'s builder
/// DROPS a `p` row naming the signer, and a NIP-29 self-grant is exactly that
/// row. `nmp_grammar::EventBuilder` normalises nothing, so the rows NMP
/// composed must survive the conversion unchanged.
pub fn as_nostr(builder: nmp::EventBuilder) -> EventBuilder {
    EventBuilder::new(builder.kind, builder.content)
        .tags(builder.tags)
        .allow_self_tagging()
}

/// kind:9002 edit-metadata that locks `group` `closed` (only members may write)
/// while keeping it `public`, and gives it the display `name`. The avatar seed
/// is the durable group id, so a rename never moves the avatar. A workspace
/// root passes its slug as both.
///
/// This applies to a subgroup exactly as it does to a root, because a
/// subgroup's `parent` is NOT metadata: it is stated once on the kind:9007
/// create (`nmp_nip29::create_group`) and cannot be restated. The only relay
/// that implements subgroups reads `parent` on the create and ignores the row
/// entirely on a kind:9002, so this edit neither sets nor clears it — the
/// relationship simply survives untouched.
pub fn group_lock_closed(group: &str, name: &str) -> nmp::EventBuilder {
    nmp_nip29::edit_metadata(workspace_visibility(name, group))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fabric::nip29::wire::KIND_GROUP_EDIT_METADATA;
    use nostr::{Event, Keys};

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

    /// Nothing this module states carries the group context row: it is NMP's
    /// group door that mints it, and a draft carrying its own would be refused
    /// there (`GroupContextError::CallerSuppliedContext`).
    #[test]
    fn no_lifecycle_draft_writes_its_own_group_context_row() {
        let keys = Keys::generate();
        for builder in [
            group_lock_closed("mosaico", "mosaico"),
            group_lock_closed("child", "Child"),
        ] {
            let event = as_nostr(builder).sign_with_keys(&keys).unwrap();
            assert!(!has_tag_name(&event, "h"), "{:?}", event.tags);
        }
    }

    #[test]
    fn group_lock_closed_is_closed_and_public() {
        let ev = as_nostr(group_lock_closed("mosaico", "mosaico"))
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert_eq!(ev.kind.as_u16(), KIND_GROUP_EDIT_METADATA);
        assert!(has_tag(&ev, "name", "mosaico"));
        assert!(has_tag_name(&ev, "closed"));
        assert!(has_tag_name(&ev, "public"));
        // Public is product policy; authenticated reads also support private groups.
        assert!(!has_tag_name(&ev, "private"));
        assert!(has_tag(
            &ev,
            "picture",
            "https://api.dicebear.com/10.x/stripes/svg?seed=mosaico"
        ));
    }

    #[test]
    fn a_subgroup_lock_names_it_and_seeds_the_avatar_from_the_group_id() {
        let ev = as_nostr(group_lock_closed(
            "subgroup-support-a1b2c3d4",
            "subgroup support",
        ))
        .sign_with_keys(&Keys::generate())
        .unwrap();
        assert_eq!(ev.kind.as_u16(), KIND_GROUP_EDIT_METADATA);
        assert!(has_tag(&ev, "name", "subgroup support"));
        assert!(has_tag_name(&ev, "closed"));
        assert!(has_tag_name(&ev, "public"));
        assert!(!has_tag_name(&ev, "private"));
        assert!(has_tag(
            &ev,
            "picture",
            "https://api.dicebear.com/10.x/stripes/svg?seed=subgroup-support-a1b2c3d4"
        ));
    }

    /// A kind:9002 must never carry `parent`. Mosaico wrote one for months and
    /// the relay discarded every single one: the row is not read by the 9002
    /// parser at all, so an app that emits it believes it re-parented a group
    /// it did not touch. The relationship rides on the kind:9007 create and
    /// nowhere else. This is the falsifier that stops the row coming back.
    #[test]
    fn a_metadata_edit_never_carries_the_subgroup_parent_row() {
        let keys = Keys::generate();
        for builder in [
            group_lock_closed("mosaico", "mosaico"),
            group_lock_closed("child", "Child"),
        ] {
            let event = as_nostr(builder).sign_with_keys(&keys).unwrap();
            assert!(!has_tag_name(&event, "parent"), "{:?}", event.tags);
        }
    }

    /// `nostr`'s builder drops a `p` row naming the signer unless self-tagging
    /// is allowed, and a NIP-29 admin self-grant is exactly that row. The
    /// conversion out of NMP's builder must not lose it.
    #[test]
    fn converting_an_nmp_draft_preserves_a_p_row_naming_the_signer() {
        let keys = Keys::generate();
        let event = as_nostr(nmp_nip29::add_user(keys.public_key(), Some("admin")))
            .sign_with_keys(&keys)
            .unwrap();
        assert!(event.tags.iter().any(|t| {
            let s = t.as_slice();
            s.first().map(String::as_str) == Some("p")
                && s.get(1).map(String::as_str) == Some(keys.public_key().to_hex().as_str())
                && s.get(2).map(String::as_str) == Some("admin")
        }));
    }
}
