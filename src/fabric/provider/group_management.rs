use super::Nip29Provider;
use crate::fabric::group_management::{
    GroupMutationOutcome, GroupOperationError, GroupOperationStage, GroupPublishOutcome,
};
use nostr::{prelude::Keys, EventBuilder};

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
            .nmp
            .add_group_users_and_wait(group, users, &user_keys)
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

    pub(in crate::fabric::provider) async fn publish_group_management(
        &self,
        group: &str,
        builder: EventBuilder,
        keys: &nostr::Keys,
        label: &str,
    ) -> GroupPublishOutcome {
        self.publish_group_management_outcome(group, builder, keys, label)
            .await
    }

    async fn publish_group_management_outcome(
        &self,
        group: &str,
        builder: EventBuilder,
        keys: &nostr::Keys,
        label: &str,
    ) -> GroupPublishOutcome {
        match self.nmp.publish_group_and_wait(group, builder, keys).await {
            Ok(_) => GroupPublishOutcome::Published,
            Err(e) => {
                let outcome = GroupPublishOutcome::Failed(GroupOperationError::from_anyhow(
                    label,
                    GroupOperationStage::Publish,
                    &e,
                ));
                let log_dir = crate::config::mosaico_home().join("logs");
                let _ = crate::config::ensure_dir(&log_dir);
                let path = log_dir.join("group-mgmt.log");
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                {
                    use std::io::Write as _;
                    let ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    let ts = crate::util::format_local_datetime_ms(ms);
                    let _ = writeln!(f, "{ts} {label} outcome={outcome:?} err={e:#}");
                }
                outcome
            }
        }
    }

    pub(crate) fn management_keys(&self) -> Option<Keys> {
        let cached = self
            .management_nsec
            .lock()
            .expect("management key mutex poisoned")
            .clone()
            .filter(|n| !n.trim().is_empty());
        if let Some(nsec) = cached {
            return match Keys::parse(&nsec) {
                Ok(keys) => Some(keys),
                Err(e) => {
                    tracing::error!(
                        error = %format!("{e:#}"),
                        "configured mosaicoPrivateKey is not parseable"
                    );
                    None
                }
            };
        }

        match crate::config::ensure_mosaico_private_key() {
            Ok(nsec) => {
                *self
                    .management_nsec
                    .lock()
                    .expect("management key mutex poisoned") = Some(nsec.clone());
                match Keys::parse(&nsec) {
                    Ok(keys) => Some(keys),
                    Err(e) => {
                        tracing::error!(
                            error = %format!("{e:#}"),
                            "persisted mosaicoPrivateKey is not parseable"
                        );
                        None
                    }
                }
            }
            Err(e) => {
                tracing::error!(
                    error = %format!("{e:#}"),
                    "failed to ensure mosaicoPrivateKey"
                );
                None
            }
        }
    }

    pub(crate) fn management_pubkey(&self) -> Option<String> {
        self.management_keys()
            .map(|keys| keys.public_key().to_hex())
    }
}
