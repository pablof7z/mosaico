use super::*;
use crate::fabric::nip29::lifecycle::{self, as_nostr};

impl Nip29Provider {
    /// Admin-set the display `name` of `group` via kind:9002 edit-metadata.
    pub async fn nip29_set_group_name(&self, group: &str, name: &str) -> bool {
        let Some(mgmt_keys) = self.management_keys() else {
            return false;
        };
        self.publish_group_management(
            group,
            as_nostr(nmp_nip29::edit_metadata(nmp_nip29::GroupMetadataEdit {
                name: Some(name.to_string()),
                ..nmp_nip29::GroupMetadataEdit::default()
            })),
            &mgmt_keys,
            "9002 edit-metadata (name)",
        )
        .await
        .is_published()
    }

    pub(in crate::fabric::provider) async fn nip29_create_root_outcome(
        &self,
        group: &str,
    ) -> GroupPublishOutcome {
        let Some(keys) = self.management_keys() else {
            return configuration_failure("9007 create-group");
        };
        let create = self
            .publish_group_management_outcome(
                group,
                as_nostr(nmp_nip29::create_group(None)),
                &keys,
                "9007 create-group",
            )
            .await;
        if let GroupPublishOutcome::Failed(_) = create {
            return create;
        }
        self.publish_group_management_outcome(
            group,
            as_nostr(lifecycle::group_lock_closed(group, group)),
            &keys,
            "9002 lock-closed",
        )
        .await
    }

    /// Create `child` as a SUBGROUP of `parent`, then lock it down.
    ///
    /// The parent is stated on the kind:9007 create and ONLY there. NMP's
    /// `create_group(Some(parent))` composes the row; the relay validates it
    /// there (the parent exists, the signer administers it, no cycle) and
    /// derives the reciprocal `child` row on the parent's kind:39000 from that
    /// same create. The kind:9002 that follows carries visibility and a display
    /// name and says nothing about ancestry — a `parent` row on a 9002 is
    /// ignored by the relay outright, so the lock is identical to a root's.
    pub(in crate::fabric::provider) async fn nip29_create_subgroup_outcome(
        &self,
        child: &str,
        name: &str,
        parent: &str,
    ) -> GroupPublishOutcome {
        let Some(keys) = self.management_keys() else {
            return configuration_failure("9007 create-subgroup");
        };
        let create = self
            .publish_group_management_outcome(
                child,
                as_nostr(nmp_nip29::create_group(Some(parent))),
                &keys,
                "9007 create-subgroup",
            )
            .await;
        if let GroupPublishOutcome::Failed(_) = create {
            return create;
        }
        self.publish_group_management_outcome(
            child,
            as_nostr(lifecycle::group_lock_closed(child, name)),
            &keys,
            "9002 lock-closed",
        )
        .await
    }
}

fn configuration_failure(operation: &str) -> GroupPublishOutcome {
    GroupPublishOutcome::Failed(GroupOperationError::new(
        operation,
        GroupOperationStage::Configuration,
        "management signing key unavailable",
    ))
}
