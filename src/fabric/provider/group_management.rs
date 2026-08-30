use super::Nip29Provider;
use crate::fabric::group_management::{
    GroupMutationOutcome, GroupOperationError, GroupOperationStage, GroupPublishOutcome,
};
use nostr::key::Keys;

mod roles;
mod topology;

impl Nip29Provider {
    pub(in crate::fabric::provider) async fn try_grant_admins_via_user_nsec(
        &self,
        group: &str,
        pubkeys: &[String],
    ) -> GroupMutationOutcome {
        let nsec = match &self.user_nsec {
            Some(n) => n.clone(),
            None => {
                eprintln!("[daemon] try_grant_admins: no userNsec configured");
                return GroupMutationOutcome::Failed(GroupOperationError::new(
                    "management self-grant",
                    GroupOperationStage::Configuration,
                    "no userNsec configured",
                ));
            }
        };
        let user_keys = match Keys::parse(&nsec) {
            Ok(k) => k,
            Err(e) => {
                eprintln!("[daemon] try_grant_admins: userNsec parse failed: {e}");
                return GroupMutationOutcome::Failed(GroupOperationError::new(
                    "management self-grant",
                    GroupOperationStage::Configuration,
                    e,
                ));
            }
        };

        let operation = "9000 put-admins (authority bootstrap via userNsec)";
        let users = match roles::group_users(pubkeys, Some("admin".into())) {
            Ok(users) => users,
            Err(error) => {
                return GroupMutationOutcome::Failed(GroupOperationError::new(
                    operation,
                    GroupOperationStage::Build,
                    error,
                ));
            }
        };

        match self
            .fava
            .add_group_users_and_wait(group, users, user_keys.public_key())
            .await
        {
            Ok(_) => GroupMutationOutcome::Published,
            Err(error) => GroupMutationOutcome::Failed(GroupOperationError::from_anyhow(
                operation,
                GroupOperationStage::Publish,
                &error,
            )),
        }
    }

    pub(crate) fn management_pubkey(&self) -> String {
        // Fava owns the backend publication signer. Public identity reads use
        // the exact signer admitted into that engine; they never touch secret
        // configuration or create secp256k1 key material.
        self.fava.backend_pubkey().to_hex()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use crate::fava_host::FavaHost;
    use crate::state::Store;

    #[tokio::test]
    async fn management_pubkey_comes_from_the_fava_registered_signer() {
        let backend = Keys::generate();
        let fava = Arc::new(FavaHost::open(&[], None, None, &backend).unwrap());
        let provider = Nip29Provider::new(
            fava,
            Arc::new(Mutex::new(Store::open_memory().unwrap())),
            None,
            Vec::new(),
        );

        assert_eq!(provider.management_pubkey(), backend.public_key().to_hex());
    }
}
