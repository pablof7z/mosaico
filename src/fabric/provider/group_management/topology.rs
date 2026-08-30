use super::*;
use crate::fabric::nip29::lifecycle;

impl Nip29Provider {
    /// Admin-set the display `name` of `group` via kind:9002 edit-metadata.
    pub async fn nip29_set_group_name(&self, group: &str, name: &str) -> bool {
        let Some(mgmt_keys) = self.management_keys() else {
            return false;
        };
        let edit = fava_simple_groups::MetadataEdit {
            name: Some(name.to_string()),
            ..Default::default()
        };
        self.fava
            .publish_metadata_edit_and_wait(group, &edit, mgmt_keys.public_key())
            .await
            .is_ok()
    }

    pub(in crate::fabric::provider) async fn nip29_create_root_outcome(
        &self,
        group: &str,
    ) -> GroupPublishOutcome {
        let Some(keys) = self.management_keys() else {
            return configuration_failure("9007 create-group");
        };
        if let Err(e) = self
            .fava
            .publish_create_group_and_wait(group, keys.public_key())
            .await
        {
            return GroupPublishOutcome::Failed(GroupOperationError::from_anyhow(
                "9007 create-group",
                GroupOperationStage::Publish,
                &e,
            ));
        }
        let edit = lifecycle::group_lock_closed(group, group);
        match self
            .fava
            .publish_metadata_edit_and_wait(group, &edit, keys.public_key())
            .await
        {
            Ok(_) => GroupPublishOutcome::Published,
            Err(e) => GroupPublishOutcome::Failed(GroupOperationError::from_anyhow(
                "9002 lock-closed",
                GroupOperationStage::Publish,
                &e,
            )),
        }
    }

    /// Create `child` as a SUBGROUP of `parent`, then lock it down.
    ///
    /// The parent is stated on the kind:9007 create and ONLY there. Fava's
    /// `publish_create_subgroup_and_wait` composes the row; the relay validates it
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
        if let Err(e) = self
            .fava
            .publish_create_subgroup_and_wait(child, parent, keys.public_key())
            .await
        {
            return GroupPublishOutcome::Failed(GroupOperationError::from_anyhow(
                "9007 create-subgroup",
                GroupOperationStage::Publish,
                &e,
            ));
        }
        let edit = lifecycle::group_lock_closed(child, name);
        match self
            .fava
            .publish_metadata_edit_and_wait(child, &edit, keys.public_key())
            .await
        {
            Ok(_) => GroupPublishOutcome::Published,
            Err(e) => GroupPublishOutcome::Failed(GroupOperationError::from_anyhow(
                "9002 lock-closed",
                GroupOperationStage::Publish,
                &e,
            )),
        }
    }
}

fn configuration_failure(operation: &str) -> GroupPublishOutcome {
    GroupPublishOutcome::Failed(GroupOperationError::new(
        operation,
        GroupOperationStage::Configuration,
        "management signing key unavailable",
    ))
}
