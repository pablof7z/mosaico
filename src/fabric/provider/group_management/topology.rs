use super::*;

impl Nip29Provider {
    /// Admin-set the display `name` of `group` via kind:9002 edit-metadata.
    pub async fn nip29_set_group_name(&self, group: &str, name: &str) -> bool {
        let Some(mgmt_keys) = self.management_keys() else {
            return false;
        };
        match crate::fabric::nip29::lifecycle::group_edit_name(group, name) {
            Ok(builder) => self
                .publish_group_management(builder, &mgmt_keys, "9002 edit-metadata (name)")
                .await
                .is_applied(),
            Err(error) => {
                tracing::error!(group, name, error = %format!("{error:#}"), "group name build failed");
                false
            }
        }
    }

    pub(in crate::fabric::provider) async fn nip29_create_root_outcome(
        &self,
        group: &str,
    ) -> GroupPublishOutcome {
        let Some(keys) = self.management_keys() else {
            return configuration_failure("9007 create-group");
        };
        let create = match crate::fabric::nip29::lifecycle::group_create(group) {
            Ok(builder) => {
                self.publish_group_management_outcome(builder, &keys, "9007 create-group")
                    .await
            }
            Err(error) => build_failure("9007 create-group", error),
        };
        if let GroupPublishOutcome::Failed(_) = create {
            return create;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        match crate::fabric::nip29::lifecycle::group_lock_closed(group) {
            Ok(builder) => {
                self.publish_group_management_outcome(builder, &keys, "9002 lock-closed")
                    .await
            }
            Err(error) => build_failure("9002 lock-closed", error),
        }
    }

    pub(in crate::fabric::provider) async fn nip29_create_subgroup_outcome(
        &self,
        child: &str,
        name: &str,
        parent: &str,
    ) -> GroupPublishOutcome {
        let Some(keys) = self.management_keys() else {
            return configuration_failure("9007 create-subgroup");
        };
        let create = match crate::fabric::nip29::lifecycle::group_create_subgroup(child, parent) {
            Ok(builder) => {
                self.publish_group_management_outcome(builder, &keys, "9007 create-subgroup")
                    .await
            }
            Err(error) => build_failure("9007 create-subgroup", error),
        };
        if let GroupPublishOutcome::Failed(_) = create {
            return create;
        }
        match crate::fabric::nip29::lifecycle::group_lock_closed_with_parent(child, name, parent) {
            Ok(builder) => {
                self.publish_group_management_outcome(builder, &keys, "9002 lock-with-parent")
                    .await
            }
            Err(error) => build_failure("9002 lock-with-parent", error),
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

fn build_failure(operation: &str, error: anyhow::Error) -> GroupPublishOutcome {
    GroupPublishOutcome::Failed(GroupOperationError::new(
        operation,
        GroupOperationStage::Build,
        error,
    ))
}
