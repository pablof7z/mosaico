use super::*;
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
        self.publish_role_changes(
            channel,
            &[pubkey_hex.to_string()],
            "member",
            "9000 put-user (session)",
            None,
        )
        .await
    }

    pub(crate) async fn nip29_add_admins_outcome(
        &self,
        channel: &str,
        pubkeys: &[String],
    ) -> GroupPublishOutcome {
        self.publish_role_changes(
            channel,
            pubkeys,
            "admin",
            "9000 put-users (admins)",
            Some("admin".to_string()),
        )
        .await
    }

    /// kind:9000 put-user, composed and contextualized by NMP's
    /// group door. `role` is `None` for a plain member and `Some(label)` for an
    /// elevated one; the label rides on the `p` row, which is NIP-29's shape
    /// and not Mosaico's to decide.
    async fn publish_role_changes(
        &self,
        channel: &str,
        pubkeys: &[String],
        role_log: &str,
        operation: &str,
        role: Option<String>,
    ) -> GroupPublishOutcome {
        let Some(mgmt_keys) = self.management_keys() else {
            return GroupPublishOutcome::Failed(GroupOperationError::new(
                operation,
                GroupOperationStage::Configuration,
                "management signing key unavailable",
            ));
        };
        let users = match group_users(pubkeys, role) {
            Ok(users) => users,
            Err(error) => {
                return GroupPublishOutcome::Failed(GroupOperationError::new(
                    operation,
                    GroupOperationStage::Build,
                    error,
                ));
            }
        };
        for pubkey in pubkeys {
            Self::log_group_role_decision(channel, pubkey, role_log, "add role");
        }
        match self
            .nmp
            .add_group_users_and_wait(channel, users, &mgmt_keys)
            .await
        {
            Ok(_) => GroupPublishOutcome::Published,
            Err(error) => GroupPublishOutcome::Failed(GroupOperationError::from_anyhow(
                operation,
                GroupOperationStage::Publish,
                &error,
            )),
        }
    }

    pub(crate) async fn nip29_remove_members_outcome(
        &self,
        channel: &str,
        pubkeys: &[String],
    ) -> GroupPublishOutcome {
        let operation = "9001 remove-users";
        let Some(mgmt_keys) = self.management_keys() else {
            return GroupPublishOutcome::Failed(GroupOperationError::new(
                operation,
                GroupOperationStage::Configuration,
                "management signing key unavailable",
            ));
        };
        let subjects = match group_pubkeys(pubkeys) {
            Ok(subjects) => subjects,
            Err(error) => {
                return GroupPublishOutcome::Failed(GroupOperationError::new(
                    operation,
                    GroupOperationStage::Build,
                    error,
                ))
            }
        };
        for pubkey in pubkeys {
            Self::log_group_role_decision(channel, pubkey, "member", "remove member");
        }
        match self
            .nmp
            .remove_group_users_and_wait(channel, subjects, &mgmt_keys)
            .await
        {
            Ok(_) => GroupPublishOutcome::Published,
            Err(error) => GroupPublishOutcome::Failed(GroupOperationError::from_anyhow(
                operation,
                GroupOperationStage::Publish,
                &error,
            )),
        }
    }
}

fn group_pubkeys(pubkeys: &[String]) -> anyhow::Result<Vec<PublicKey>> {
    if pubkeys.is_empty() {
        anyhow::bail!("a group role change must name at least one user");
    }
    pubkeys
        .iter()
        .map(|pubkey| PublicKey::parse(pubkey).map_err(anyhow::Error::from))
        .collect()
}

pub(super) fn group_users(
    pubkeys: &[String],
    role: Option<String>,
) -> anyhow::Result<Vec<nmp::nip29::GroupUser>> {
    if pubkeys.is_empty() {
        anyhow::bail!("a group role change must name at least one user");
    }
    pubkeys
        .iter()
        .map(|pubkey| {
            PublicKey::parse(pubkey)
                .map(|pubkey| nmp::nip29::GroupUser::new(pubkey, role.clone()))
                .map_err(anyhow::Error::from)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    #[test]
    fn all_parent_admins_form_one_group_user_batch() {
        let first = Keys::generate().public_key();
        let second = Keys::generate().public_key();
        let users = group_users(&[first.to_hex(), second.to_hex()], Some("admin".into())).unwrap();

        assert_eq!(users.len(), 2);
        assert_eq!(
            users[0],
            nmp::nip29::GroupUser::new(first, Some("admin".into()))
        );
        assert_eq!(
            users[1],
            nmp::nip29::GroupUser::new(second, Some("admin".into()))
        );
    }

    #[test]
    fn all_archive_removals_form_one_pubkey_batch() {
        let first = Keys::generate().public_key();
        let second = Keys::generate().public_key();

        assert_eq!(
            group_pubkeys(&[first.to_hex(), second.to_hex()]).unwrap(),
            vec![first, second]
        );
    }
}
