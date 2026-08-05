//! Group-lifecycle drafts NIP-29 does not compose for us.
//!
//! NIP-29's own schemas -- create-group, delete-group, put-user, remove-user
//! and edit-metadata's `name`/`about` -- belong to `nmp_nip29::operations` and
//! are called there directly. Nothing in this module re-spells them, and
//! nothing here writes an `h` row: the group context tag is minted by NMP's
//! group door at publish time.
//!
//! What is left are the three drafts NMP's composers cannot currently express:
//!
//! * **Visibility.** NIP-29's kind:9002 carries `open`/`closed` and
//!   `public`/`private` alongside `name`/`about`, and `nmp_nip29::edit_metadata`
//!   composes only the latter two. A Mosaico workspace is a CLOSED group, so
//!   the flags are not optional decoration.
//! * **`picture`.** Also a NIP-29 kind:9002 metadata field, also uncomposed.
//!   (The dicebear URL is Mosaico product policy; the tag name is NIP-29's.)
//! * **`parent`.** Subgroups per nostr-protocol/nips#2319: the relationship
//!   rides on the kind:9007 create and the relay re-emits it on kind:39000.
//!
//! Tracked upstream as pablof7z/nmp#1282. Each draft below starts from the
//! `nmp_nip29` verb that owns its kind and appends only the rows NMP has no
//! spelling for, so the gap stays exactly as wide as it really is.

use anyhow::Result;
use nostr::*;

fn tag(parts: &[&str]) -> Result<Tag> {
    Ok(Tag::parse(parts.iter().copied())?)
}

fn picture_tag(seed: &str) -> Result<Tag> {
    let url = format!("https://api.dicebear.com/10.x/stripes/svg?seed={seed}");
    tag(&["picture", &url])
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

/// kind:9002 edit-metadata that locks the group `closed` (only members may write)
/// while keeping it `public`. The workspace is the root channel, so its visible
/// name and durable group id use the same workspace slug.
pub fn group_lock_closed(channel: &str) -> Result<nmp::EventBuilder> {
    Ok(nmp_nip29::edit_metadata(Some(channel), None)
        .tag(tag(&["closed"])?)
        .tag(tag(&["public"])?)
        .tag(picture_tag(channel)?))
}

/// kind:9007 create-group for a CHILD (sub-)group, declaring its `parent`
/// relationship at creation. NIP-29 subgroup relays (per
/// nostr-protocol/nips#2319, e.g. nip29.f7z.io) validate the parent at create
/// time (parent must exist; signer must be a parent admin; no cycles) and
/// re-emit the tag on the relay-authored kind:39000. The signer becomes the
/// subgroup admin and, as with any fresh group, it is OPEN until locked.
pub fn group_create_subgroup(parent_h: &str) -> Result<nmp::EventBuilder> {
    Ok(nmp_nip29::create_group().tag(tag(&["parent", parent_h])?))
}

/// kind:9002 edit-metadata that locks a CHILD group `closed` while keeping it
/// `public` (the current product visibility policy) AND declares its NIP-29
/// subgroup parent. Unlike [`group_lock_closed`], `name` is a human-readable
/// display name rather than the slug. Shared acquisition follows this public
/// visibility policy; closed membership still governs writes.
pub fn group_lock_closed_with_parent(
    child_h: &str,
    name: &str,
    parent_h: &str,
) -> Result<nmp::EventBuilder> {
    Ok(nmp_nip29::edit_metadata(Some(name), None)
        .tag(tag(&["parent", parent_h])?)
        .tag(tag(&["closed"])?)
        .tag(tag(&["public"])?)
        .tag(picture_tag(child_h)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fabric::nip29::wire::{KIND_GROUP_CREATE, KIND_GROUP_EDIT_METADATA};

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

    /// Every draft in this module leaves the group context row to NMP's group
    /// door. A draft that carried its own would be refused there
    /// (`GroupContextError::CallerSuppliedContext`), so this is the falsifier
    /// for the whole module rather than a style check.
    #[test]
    fn no_lifecycle_draft_writes_its_own_group_context_row() {
        let keys = Keys::generate();
        for builder in [
            group_lock_closed("mosaico").unwrap(),
            group_create_subgroup("mosaico").unwrap(),
            group_lock_closed_with_parent("child", "Child", "mosaico").unwrap(),
        ] {
            let event = as_nostr(builder).sign_with_keys(&keys).unwrap();
            assert!(!has_tag_name(&event, "h"), "{:?}", event.tags);
        }
    }

    #[test]
    fn group_lock_closed_is_closed_and_public() {
        let ev = as_nostr(group_lock_closed("mosaico").unwrap())
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
    fn group_create_subgroup_declares_its_parent() {
        let ev = as_nostr(group_create_subgroup("mosaico").unwrap())
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert_eq!(ev.kind.as_u16(), KIND_GROUP_CREATE);
        // The parent relationship must ride on the 9007 create (NIP #2319 relays
        // validate + re-emit it on 39000 from the create event).
        assert!(has_tag(&ev, "parent", "mosaico"));
    }

    #[test]
    fn subgroup_lock_has_parent_name_closed_public() {
        let ev = as_nostr(
            group_lock_closed_with_parent(
                "subgroup-support-a1b2c3d4",
                "subgroup support",
                "mosaico",
            )
            .unwrap(),
        )
        .sign_with_keys(&Keys::generate())
        .unwrap();
        assert_eq!(ev.kind.as_u16(), KIND_GROUP_EDIT_METADATA);
        assert!(has_tag(&ev, "name", "subgroup support"));
        assert!(has_tag(&ev, "parent", "mosaico"));
        assert!(has_tag_name(&ev, "closed"));
        assert!(has_tag_name(&ev, "public"));
        assert!(!has_tag_name(&ev, "private"));
        assert!(has_tag(
            &ev,
            "picture",
            "https://api.dicebear.com/10.x/stripes/svg?seed=subgroup-support-a1b2c3d4"
        ));
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
