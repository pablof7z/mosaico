use super::Nip29Provider;
use crate::fabric::group_management::{GroupMutationOutcome, GroupPublishOutcome};
use nostr::key::Keys;

impl Nip29Provider {
    pub(crate) async fn grant_member_published(
        &self,
        channel: &str,
        pubkey: &str,
        keys: &Keys,
    ) -> GroupMutationOutcome {
        published(self.nip29_add_member_outcome(channel, pubkey, keys).await)
    }

    pub(crate) async fn grant_admin_published(
        &self,
        channel: &str,
        pubkey: &str,
        keys: &Keys,
    ) -> GroupMutationOutcome {
        self.grant_admins_published(channel, &[pubkey.to_string()], keys)
            .await
    }

    /// Publish every missing administrator in one kind:9000 event and await
    /// Nostr runtime's one terminal receipt result. No local roster polling and no
    /// repeated publication are part of this contract.
    pub(crate) async fn grant_admins_published(
        &self,
        channel: &str,
        pubkeys: &[String],
        keys: &Keys,
    ) -> GroupMutationOutcome {
        published(self.nip29_add_admins_outcome(channel, pubkeys, keys).await)
    }

    pub(crate) async fn remove_member_published(
        &self,
        channel: &str,
        pubkey: &str,
        keys: &Keys,
    ) -> GroupMutationOutcome {
        self.remove_members_published(channel, &[pubkey.to_string()], keys)
            .await
    }

    /// Publish every removal in one kind:9001 event and await one result.
    pub(crate) async fn remove_members_published(
        &self,
        channel: &str,
        pubkeys: &[String],
        keys: &Keys,
    ) -> GroupMutationOutcome {
        published(
            self.nip29_remove_members_outcome(channel, pubkeys, keys)
                .await,
        )
    }
}

fn published(outcome: GroupPublishOutcome) -> GroupMutationOutcome {
    match outcome {
        GroupPublishOutcome::Published => GroupMutationOutcome::Published,
        GroupPublishOutcome::Failed(error) => GroupMutationOutcome::Failed(error),
    }
}
