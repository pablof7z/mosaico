use super::Nip29Provider;
use crate::fabric::group_management::{
    classify_group_publish_error, GroupMutationOutcome, GroupPublishOutcome,
};
use nostr_sdk::{prelude::Keys, EventBuilder};

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
                return GroupMutationOutcome::Rejected;
            }
        };
        let user_keys = match Keys::parse(&nsec) {
            Ok(k) => k,
            Err(e) => {
                eprintln!("[daemon] try_grant_mgmt_admin: userNsec parse failed: {e}");
                return GroupMutationOutcome::Rejected;
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
                    return GroupMutationOutcome::Rejected;
                }
            };
            match self.try_fetch_group_state(group).await {
                Ok((_, roles, _)) => {
                    if roles.get(mgmt_pubkey).map(String::as_str) == Some("admin") {
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
                }
                Err(e) => {
                    tracing::error!(
                        channel = group,
                        pubkey = mgmt_pubkey,
                        attempt,
                        error = %e,
                        "try_grant_mgmt_admin: relay read-back failed; cannot confirm self-grant"
                    );
                }
            }
            if outcome.is_rejected() {
                return GroupMutationOutcome::Rejected;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250 * (attempt as u64 + 1))).await;
        }
        GroupMutationOutcome::Unconfirmed
    }

    pub(in crate::fabric::provider) async fn publish_group_management(
        &self,
        builder: EventBuilder,
        keys: &nostr_sdk::prelude::Keys,
        label: &str,
    ) -> bool {
        self.publish_group_management_outcome(builder, keys, label)
            .await
            .is_applied()
    }

    async fn publish_group_management_outcome(
        &self,
        builder: EventBuilder,
        keys: &nostr_sdk::prelude::Keys,
        label: &str,
    ) -> GroupPublishOutcome {
        match self.transport.publish_signed_checked(builder, keys).await {
            Ok(_) => GroupPublishOutcome::Applied,
            Err(e) => {
                let s = e.to_string();
                let outcome = classify_group_publish_error(&s);
                let log_dir = crate::config::edge_home().join("logs");
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
                        "configured tenexPrivateKey is not parseable"
                    );
                    None
                }
            };
        }

        match crate::config::ensure_tenex_private_key() {
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
                            "persisted tenexPrivateKey is not parseable"
                        );
                        None
                    }
                }
            }
            Err(e) => {
                tracing::error!(
                    error = %format!("{e:#}"),
                    "failed to ensure tenexPrivateKey"
                );
                None
            }
        }
    }

    pub(crate) fn management_pubkey(&self) -> Option<String> {
        self.management_keys()
            .map(|keys| keys.public_key().to_hex())
    }

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
        let Some(mgmt_keys) = self.management_keys() else {
            return GroupPublishOutcome::Rejected;
        };
        Self::log_group_role_decision(channel, pubkey_hex, "member", "add member");
        match crate::fabric::nip29::lifecycle::group_put_user(channel, pubkey_hex) {
            Ok(b) => {
                self.publish_group_management_outcome(b, &mgmt_keys, "9000 put-user (session)")
                    .await
            }
            Err(e) => {
                tracing::error!(
                    group = channel,
                    pubkey = pubkey_hex,
                    error = %format!("{e:#}"),
                    "nip29_add_member: group_put_user build failed — failing closed"
                );
                GroupPublishOutcome::Rejected
            }
        }
    }

    /// Admin-set the display `name` of `group` via kind:9002 edit-metadata.
    pub async fn nip29_set_group_name(&self, group: &str, name: &str) -> bool {
        let Some(mgmt_keys) = self.management_keys() else {
            return false;
        };
        eprintln!("[daemon] nip29 set-name h={group} name={name:?}");
        match crate::fabric::nip29::lifecycle::group_edit_name(group, name) {
            Ok(b) => {
                self.publish_group_management(b, &mgmt_keys, "9002 edit-metadata (name)")
                    .await
            }
            Err(e) => {
                tracing::error!(
                    group,
                    name,
                    error = %format!("{e:#}"),
                    "nip29_set_group_name: group_edit_name build failed — failing closed"
                );
                false
            }
        }
    }

    pub(crate) async fn nip29_add_admin_outcome(
        &self,
        channel: &str,
        pubkey_hex: &str,
    ) -> GroupPublishOutcome {
        let Some(mgmt_keys) = self.management_keys() else {
            return GroupPublishOutcome::Rejected;
        };
        Self::log_group_role_decision(channel, pubkey_hex, "admin", "add admin");
        match crate::fabric::nip29::lifecycle::group_put_admin(channel, pubkey_hex) {
            Ok(b) => {
                self.publish_group_management_outcome(b, &mgmt_keys, "9000 put-user (admin)")
                    .await
            }
            Err(e) => {
                tracing::error!(
                    group = channel,
                    pubkey = pubkey_hex,
                    error = %format!("{e:#}"),
                    "nip29_add_admin: group_put_admin build failed — failing closed"
                );
                GroupPublishOutcome::Rejected
            }
        }
    }

    /// Create + lock a NIP-29 subgroup.
    pub async fn nip29_create_subgroup(&self, child_h: &str, name: &str, parent_h: &str) -> bool {
        let Some(mgmt_keys) = self.management_keys() else {
            return false;
        };
        eprintln!("[daemon] nip29 create-subgroup h={child_h} name={name:?} parent={parent_h}");
        let created =
            match crate::fabric::nip29::lifecycle::group_create_subgroup(child_h, parent_h) {
                Ok(b) => {
                    self.publish_group_management(b, &mgmt_keys, "9007 create-subgroup")
                        .await
                }
                Err(e) => {
                    tracing::error!(
                        child = child_h,
                        parent = parent_h,
                        error = %format!("{e:#}"),
                        "nip29_create_subgroup: group_create_subgroup build failed — failing closed"
                    );
                    false
                }
            };
        if !created {
            return false;
        }
        match crate::fabric::nip29::lifecycle::group_lock_closed_with_parent(
            child_h, name, parent_h,
        ) {
            Ok(b) => {
                self.publish_group_management(b, &mgmt_keys, "9002 lock-with-parent")
                    .await
            }
            Err(e) => {
                tracing::error!(
                    child = child_h,
                    parent = parent_h,
                    error = %format!("{e:#}"),
                    "nip29_create_subgroup: group_lock_closed_with_parent build failed — failing closed"
                );
                false
            }
        }
    }

    pub(crate) async fn nip29_remove_member_outcome(
        &self,
        channel: &str,
        pubkey_hex: &str,
    ) -> GroupPublishOutcome {
        let Some(mgmt_keys) = self.management_keys() else {
            return GroupPublishOutcome::Rejected;
        };
        eprintln!(
            "[daemon] nip29 remove-member h={channel} p={}",
            crate::util::pubkey_short(pubkey_hex)
        );
        match crate::fabric::nip29::lifecycle::group_remove_user(channel, pubkey_hex) {
            Ok(b) => {
                self.publish_group_management_outcome(b, &mgmt_keys, "9001 remove-user (session)")
                    .await
            }
            Err(e) => {
                tracing::error!(
                    group = channel,
                    pubkey = pubkey_hex,
                    error = %format!("{e:#}"),
                    "nip29_remove_member: group_remove_user build failed — failing closed"
                );
                GroupPublishOutcome::Rejected
            }
        }
    }
}
