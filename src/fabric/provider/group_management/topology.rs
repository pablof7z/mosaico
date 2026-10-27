use super::*;
use crate::fabric::group_management::{
    GroupOperationError, GroupOperationStage, GroupPublishOutcome,
};
use crate::fabric::nip29::lifecycle;
use crate::fabric::nip29::management::MetadataEdit;
use nostr::key::Keys;

impl Nip29Provider {
    pub async fn nip29_set_group_name(&self, group: &str, name: &str, keys: &Keys) -> bool {
        self.publish_metadata_edit(
            group,
            &MetadataEdit {
                name: Some(name.to_string()),
                ..Default::default()
            },
            keys,
        )
        .await
        .is_ok()
    }

    pub(in crate::fabric::provider) async fn nip29_create_root_outcome(
        &self,
        group: &str,
        keys: &Keys,
    ) -> GroupPublishOutcome {
        if let Err(error) = self.publish_group_create(group, None, keys).await {
            return failed("9007 create-group", error);
        }
        match self
            .publish_metadata_edit(group, &lifecycle::group_lock_closed(group, group), keys)
            .await
        {
            Ok(_) => GroupPublishOutcome::Published,
            Err(error) => failed("9002 lock-closed", error),
        }
    }

    pub(in crate::fabric::provider) async fn nip29_create_subgroup_outcome(
        &self,
        child: &str,
        name: &str,
        parent: &str,
        keys: &Keys,
    ) -> GroupPublishOutcome {
        if let Err(error) = self.publish_group_create(child, Some(parent), keys).await {
            return failed("9007 create-subgroup", error);
        }
        match self
            .publish_metadata_edit(child, &lifecycle::group_lock_closed(child, name), keys)
            .await
        {
            Ok(_) => GroupPublishOutcome::Published,
            Err(error) => failed("9002 lock-closed", error),
        }
    }
}

fn failed(operation: &str, error: anyhow::Error) -> GroupPublishOutcome {
    GroupPublishOutcome::Failed(GroupOperationError::from_anyhow(
        operation,
        GroupOperationStage::Publish,
        &error,
    ))
}
