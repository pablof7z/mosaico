use super::*;
use crate::fabric::group_management::{
    GroupOperationError, GroupOperationStage, GroupPublishOutcome,
};
use crate::fabric::nip29::management::GroupUser;
use nostr::key::{Keys, PublicKey};

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
        pubkey: &str,
        keys: &Keys,
    ) -> GroupPublishOutcome {
        self.publish_role_changes(
            channel,
            &[pubkey.to_string()],
            "member",
            "9000 put-user (session)",
            None,
            keys,
        )
        .await
    }

    pub(crate) async fn nip29_add_admins_outcome(
        &self,
        channel: &str,
        pubkeys: &[String],
        keys: &Keys,
    ) -> GroupPublishOutcome {
        self.publish_role_changes(
            channel,
            pubkeys,
            "admin",
            "9000 put-users (admins)",
            Some("admin".to_string()),
            keys,
        )
        .await
    }

    async fn publish_role_changes(
        &self,
        channel: &str,
        pubkeys: &[String],
        role_log: &str,
        operation: &str,
        role: Option<String>,
        keys: &Keys,
    ) -> GroupPublishOutcome {
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
        match self.publish_group_users(channel, &users, keys).await {
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
        keys: &Keys,
    ) -> GroupPublishOutcome {
        let operation = "9001 remove-users";
        let subjects = match group_pubkeys(pubkeys) {
            Ok(subjects) => subjects,
            Err(error) => {
                return GroupPublishOutcome::Failed(GroupOperationError::new(
                    operation,
                    GroupOperationStage::Build,
                    error,
                ));
            }
        };
        for pubkey in pubkeys {
            Self::log_group_role_decision(channel, pubkey, "member", "remove member");
        }
        match self.publish_group_removals(channel, &subjects, keys).await {
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
) -> anyhow::Result<Vec<GroupUser>> {
    if pubkeys.is_empty() {
        anyhow::bail!("a group role change must name at least one user");
    }
    pubkeys
        .iter()
        .map(|pubkey| {
            PublicKey::parse(pubkey)
                .map(|pubkey| GroupUser {
                    pubkey,
                    roles: role.iter().cloned().collect(),
                })
                .map_err(anyhow::Error::from)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_parent_admins_form_one_group_user_batch() {
        let first = Keys::generate().public_key();
        let second = Keys::generate().public_key();
        let users = group_users(&[first.to_hex(), second.to_hex()], Some("admin".into())).unwrap();
        assert_eq!(users[0].pubkey, first);
        assert_eq!(users[1].pubkey, second);
        assert!(users.iter().all(|user| user.roles == ["admin"]));
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
