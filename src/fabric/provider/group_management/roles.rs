use super::*;
use crate::fabric::nip29::lifecycle::as_nostr;
use nostr::PublicKey;

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
            None,
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
            Some("admin"),
        )
        .await
    }

    /// kind:9000 put-user, composed by `nmp_nip29` and contextualized by NMP's
    /// group door. `role` is `None` for a plain member and `Some(label)` for an
    /// elevated one; the label rides on the `p` row, which is NIP-29's shape
    /// and not Mosaico's to decide.
    async fn publish_role_change(
        &self,
        channel: &str,
        pubkey_hex: &str,
        role_log: &str,
        operation: &str,
        role: Option<&str>,
    ) -> GroupPublishOutcome {
        let Some(mgmt_keys) = self.management_keys() else {
            return GroupPublishOutcome::Failed(GroupOperationError::new(
                operation,
                GroupOperationStage::Configuration,
                "management signing key unavailable",
            ));
        };
        let subject = match PublicKey::parse(pubkey_hex) {
            Ok(subject) => subject,
            Err(error) => {
                return GroupPublishOutcome::Failed(GroupOperationError::new(
                    operation,
                    GroupOperationStage::Build,
                    error,
                ))
            }
        };
        Self::log_group_role_decision(channel, pubkey_hex, role_log, "add role");
        self.publish_group_management_outcome(
            channel,
            as_nostr(nmp_nip29::add_user(subject, role)),
            &mgmt_keys,
            operation,
        )
        .await
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
        let subject = match PublicKey::parse(pubkey_hex) {
            Ok(subject) => subject,
            Err(error) => {
                return GroupPublishOutcome::Failed(GroupOperationError::new(
                    operation,
                    GroupOperationStage::Build,
                    error,
                ))
            }
        };
        Self::log_group_role_decision(channel, pubkey_hex, "member", "remove member");
        self.publish_group_management_outcome(
            channel,
            as_nostr(nmp_nip29::remove_user(subject)),
            &mgmt_keys,
            operation,
        )
        .await
    }
}
