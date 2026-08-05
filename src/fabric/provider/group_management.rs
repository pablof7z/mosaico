use super::Nip29Provider;
use crate::fabric::group_management::{
    GroupMutationOutcome, GroupOperationError, GroupOperationStage, GroupPublishOutcome,
};
use nostr::{prelude::Keys, EventBuilder};

mod roles;
mod topology;

impl Nip29Provider {
    pub(in crate::fabric::provider) async fn try_grant_mgmt_admin_via_user_nsec(
        &self,
        group: &str,
        mgmt_pubkey: &str,
    ) -> GroupMutationOutcome {
        let nsec = match &self.user_nsec {
            Some(n) => n.clone(),
            None => {
                eprintln!("[daemon] try_grant_mgmt_admin: no userNsec configured");
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
                eprintln!("[daemon] try_grant_mgmt_admin: userNsec parse failed: {e}");
                return GroupMutationOutcome::Failed(GroupOperationError::new(
                    "management self-grant",
                    GroupOperationStage::Configuration,
                    e,
                ));
            }
        };

        for attempt in 0..6u32 {
            let outcome = match crate::fabric::nip29::lifecycle::group_put_admin(group, mgmt_pubkey)
            {
                Ok(b) => {
                    self.publish_group_management_outcome(
                        b,
                        &user_keys,
                        "9000 put-admin (self-grant via userNsec)",
                    )
                    .await
                }
                Err(e) => {
                    eprintln!("[daemon] try_grant_mgmt_admin: build event failed: {e}");
                    return GroupMutationOutcome::Failed(GroupOperationError::new(
                        "9000 put-admin (self-grant via userNsec)",
                        GroupOperationStage::Build,
                        e,
                    ));
                }
            };
            // Confirmed by presence on the relay's kind:39001, not by that
            // record spelling the role "admin" — see `confirm_role_grant`.
            match self.with_store(|s| s.is_channel_admin(group, mgmt_pubkey)) {
                Ok(true) => {
                    self.with_store(|s| {
                        if let Err(e) = s.upsert_channel_member(
                            group,
                            mgmt_pubkey,
                            "admin",
                            crate::util::now_secs(),
                        ) {
                            tracing::error!(
                                channel = group,
                                pubkey = mgmt_pubkey,
                                error = %e,
                                "try_grant_mgmt_admin: local mirror write failed after confirmed relay grant"
                            );
                        }
                    });
                    return GroupMutationOutcome::Confirmed;
                }
                Ok(false) => {}
                Err(e) => {
                    tracing::error!(
                        channel = group,
                        pubkey = mgmt_pubkey,
                        attempt,
                        error = %e,
                        "try_grant_mgmt_admin: roster read-back failed; cannot confirm self-grant"
                    );
                }
            }
            if let GroupPublishOutcome::Failed(error) = outcome {
                return GroupMutationOutcome::Failed(error);
            }
            tokio::time::sleep(std::time::Duration::from_millis(250 * (attempt as u64 + 1))).await;
        }
        GroupMutationOutcome::Unconfirmed {
            detail: "roster read-back never showed the management admin grant".into(),
        }
    }

    pub(in crate::fabric::provider) async fn publish_group_management(
        &self,
        builder: EventBuilder,
        keys: &nostr::Keys,
        label: &str,
    ) -> GroupPublishOutcome {
        self.publish_group_management_outcome(builder, keys, label)
            .await
    }

    async fn publish_group_management_outcome(
        &self,
        builder: EventBuilder,
        keys: &nostr::Keys,
        label: &str,
    ) -> GroupPublishOutcome {
        match self.nmp.publish_group_builder(builder, keys) {
            Ok(_) => GroupPublishOutcome::Applied,
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
