use super::*;

impl Nip29Provider {
    fn log_group_role_decision(channel: &str, pubkey: &str, role: &str, reason: &str) {
        eprintln!(
            "[daemon] nip29-role-decision channel={channel} target={} role={role} reason={reason}",
            crate::util::pubkey_short(pubkey)
        );
    }

    pub(crate) async fn nip29_add_member_outcome(
        &self,
        channel: &str,
        pubkey_hex: &str,
    ) -> GroupPublishOutcome {
        self.publish_role_change(
            channel,
            pubkey_hex,
            "member",
            "9000 put-user (session)",
            crate::fabric::nip29::lifecycle::group_put_user,
        )
        .await
    }

    pub(crate) async fn nip29_add_admin_outcome(
        &self,
        channel: &str,
        pubkey_hex: &str,
    ) -> GroupPublishOutcome {
        self.publish_role_change(
            channel,
            pubkey_hex,
            "admin",
            "9000 put-user (admin)",
            crate::fabric::nip29::lifecycle::group_put_admin,
        )
        .await
    }

    async fn publish_role_change(
        &self,
        channel: &str,
        pubkey_hex: &str,
        role: &str,
        operation: &str,
        build: fn(&str, &str) -> anyhow::Result<EventBuilder>,
    ) -> GroupPublishOutcome {
        let Some(mgmt_keys) = self.management_keys() else {
            return GroupPublishOutcome::Failed(GroupOperationError::new(
                operation,
                GroupOperationStage::Configuration,
                "management signing key unavailable",
            ));
        };
        Self::log_group_role_decision(channel, pubkey_hex, role, "add role");
        match build(channel, pubkey_hex) {
            Ok(builder) => {
                self.publish_group_management_outcome(builder, &mgmt_keys, operation)
                    .await
            }
            Err(error) => GroupPublishOutcome::Failed(GroupOperationError::new(
                operation,
                GroupOperationStage::Build,
                error,
            )),
        }
    }

    pub(crate) async fn nip29_remove_member_outcome(
        &self,
        channel: &str,
        pubkey_hex: &str,
    ) -> GroupPublishOutcome {
        let operation = "9001 remove-user (session)";
        let Some(mgmt_keys) = self.management_keys() else {
            return GroupPublishOutcome::Failed(GroupOperationError::new(
                operation,
                GroupOperationStage::Configuration,
                "management signing key unavailable",
            ));
        };
        Self::log_group_role_decision(channel, pubkey_hex, "member", "remove member");
        match crate::fabric::nip29::lifecycle::group_remove_user(channel, pubkey_hex) {
            Ok(builder) => {
                self.publish_group_management_outcome(builder, &mgmt_keys, operation)
                    .await
            }
            Err(error) => GroupPublishOutcome::Failed(GroupOperationError::new(
                operation,
                GroupOperationStage::Build,
                error,
            )),
        }
    }
}
