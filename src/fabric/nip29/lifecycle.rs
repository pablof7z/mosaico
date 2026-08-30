//! Mosaico's group policy, stated for NIP-29's own schemas to compose.
//!
//! Nothing in this module composes a wire row. It states what Mosaico wants —
//! the avatar URL, the visibility a workspace has — and produces a
//! `MetadataEdit` for the caller to publish via `FavaHost`.

use fava_simple_groups::{GroupAccess, MetadataEdit};

/// Mosaico's own avatar policy.
fn picture_url(seed: &str) -> String {
    format!("https://api.dicebear.com/10.x/stripes/svg?seed={seed}")
}

/// kind:9002 edit-metadata that locks `group` `closed` (only members may write)
/// while keeping it `public`, and gives it the display `name`. The avatar seed
/// is the durable group id, so a rename never moves the avatar. A workspace
/// root passes its slug as both.
///
/// This applies to a subgroup exactly as it does to a root, because a
/// subgroup's `parent` is NOT metadata: it is stated once on the kind:9007
/// create and cannot be restated.
pub fn group_lock_closed(group: &str, name: &str) -> MetadataEdit {
    MetadataEdit {
        name: Some(name.to_string()),
        picture: Some(picture_url(group)),
        access: Some(GroupAccess::Closed),
        ..MetadataEdit::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fava_simple_groups::GroupVisibility;

    #[test]
    fn group_lock_closed_is_closed_and_public() {
        let edit = group_lock_closed("mosaico", "mosaico");
        assert_eq!(edit.name.as_deref(), Some("mosaico"));
        assert_eq!(edit.access, Some(GroupAccess::Closed));
        assert!(edit.visibility.is_none() || edit.visibility == Some(GroupVisibility::Public));
        assert_eq!(
            edit.picture.as_deref(),
            Some("https://api.dicebear.com/10.x/stripes/svg?seed=mosaico")
        );
    }

    #[test]
    fn a_subgroup_lock_names_it_and_seeds_the_avatar_from_the_group_id() {
        let edit = group_lock_closed("subgroup-support-a1b2c3d4", "subgroup support");
        assert_eq!(edit.name.as_deref(), Some("subgroup support"));
        assert_eq!(edit.access, Some(GroupAccess::Closed));
        assert_eq!(
            edit.picture.as_deref(),
            Some("https://api.dicebear.com/10.x/stripes/svg?seed=subgroup-support-a1b2c3d4")
        );
    }
}
