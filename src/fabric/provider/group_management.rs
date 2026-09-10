use super::Nip29Provider;
use crate::fabric::group_management::{
    GroupMutationOutcome, GroupOperationError, GroupOperationStage,
};
use crate::fabric::nip29::management::{self, GroupUser, MetadataEdit};
use nostr::key::{Keys, PublicKey};

mod roles;
mod topology;

impl Nip29Provider {
    pub(in crate::fabric::provider) async fn try_grant_admins_via_user_nsec(
        &self,
        group: &str,
        pubkeys: &[String],
    ) -> GroupMutationOutcome {
        let Some(nsec) = self.user_nsec.as_deref() else {
            return GroupMutationOutcome::Failed(GroupOperationError::new(
                "management self-grant",
                GroupOperationStage::Configuration,
                "no userNsec configured",
            ));
        };
        let user_keys = match Keys::parse(nsec) {
            Ok(keys) => keys,
            Err(error) => {
                return GroupMutationOutcome::Failed(GroupOperationError::new(
                    "management self-grant",
                    GroupOperationStage::Configuration,
                    error,
                ));
            }
        };
        let users = match roles::group_users(pubkeys, Some("admin".to_string())) {
            Ok(users) => users,
            Err(error) => {
                return GroupMutationOutcome::Failed(GroupOperationError::new(
                    "9000 put-admins (authority bootstrap via userNsec)",
                    GroupOperationStage::Build,
                    error,
                ));
            }
        };
        let builder = match management::put_users(group, &users) {
            Ok(builder) => builder,
            Err(error) => {
                return GroupMutationOutcome::Failed(GroupOperationError::from_anyhow(
                    "9000 put-admins (authority bootstrap via userNsec)",
                    GroupOperationStage::Build,
                    &error,
                ));
            }
        };
        match self.publish_built(builder, &user_keys).await {
            Ok(_) => GroupMutationOutcome::Published,
            Err(error) => GroupMutationOutcome::Failed(GroupOperationError::from_anyhow(
                "9000 put-admins (authority bootstrap via userNsec)",
                GroupOperationStage::Publish,
                &error,
            )),
        }
    }

    pub(crate) async fn publish_metadata_edit(
        &self,
        group: &str,
        edit: &MetadataEdit,
        keys: &Keys,
    ) -> anyhow::Result<nostr::event::EventId> {
        self.publish_built(management::edit_metadata(group, edit)?, keys)
            .await
    }

    pub(crate) async fn publish_group_create(
        &self,
        group: &str,
        parent: Option<&str>,
        keys: &Keys,
    ) -> anyhow::Result<nostr::event::EventId> {
        self.publish_built(management::create_group(group, parent)?, keys)
            .await
    }

    pub(crate) async fn publish_group_delete(
        &self,
        group: &str,
        keys: &Keys,
    ) -> anyhow::Result<nostr::event::EventId> {
        self.publish_built(management::delete_group(group)?, keys)
            .await
    }

    async fn publish_group_users(
        &self,
        group: &str,
        users: &[GroupUser],
        keys: &Keys,
    ) -> anyhow::Result<()> {
        if users.is_empty() {
            return Ok(());
        }
        self.publish_built(management::put_users(group, users)?, keys)
            .await
            .map(|_| ())
    }

    async fn publish_group_removals(
        &self,
        group: &str,
        users: &[PublicKey],
        keys: &Keys,
    ) -> anyhow::Result<()> {
        if users.is_empty() {
            return Ok(());
        }
        self.publish_built(management::remove_users(group, users)?, keys)
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::fabric::nip29::view::Nip29ViewStore;
    use crate::nostr_runtime::NostrRuntime;
    use crate::state::Store;

    #[test]
    fn management_pubkey_is_the_explicit_product_identity() {
        let backend = Keys::generate();
        let runtime = Arc::new(NostrRuntime::new("wss://relay.invalid").unwrap());
        let view = Arc::new(Nip29ViewStore::default());
        let provider = Nip29Provider::new(
            runtime,
            view,
            Arc::new(Mutex::new(Store::open_memory().unwrap())),
            backend.public_key(),
            None,
            Vec::new(),
        );
        assert_eq!(provider.management_pubkey(), backend.public_key().to_hex());
    }
}
