use super::Nip29Provider;
use crate::fabric::group_management::{classify_group_publish_error, GroupPublishOutcome};
use nostr_sdk::EventBuilder;

impl Nip29Provider {
    pub(in crate::fabric::provider) async fn try_grant_mgmt_admin_via_user_nsec(
        &self,
        group: &str,
        mgmt_pubkey: &str,
    ) -> bool {
        use nostr_sdk::prelude::Keys;
        let nsec = match &self.user_nsec {
            Some(n) => n.clone(),
            None => {
                eprintln!("[daemon] try_grant_mgmt_admin: no userNsec configured");
                return false;
            }
        };
        let user_keys = match Keys::parse(&nsec) {
            Ok(k) => k,
            Err(e) => {
                eprintln!("[daemon] try_grant_mgmt_admin: userNsec parse failed: {e}");
                return false;
            }
        };
        match crate::fabric::nip29::lifecycle::group_put_admin(group, mgmt_pubkey) {
            Ok(b) => {
                self.publish_group_management(
                    b,
                    &user_keys,
                    "9000 put-admin (self-grant via userNsec)",
                )
                .await
            }
            Err(e) => {
                eprintln!("[daemon] try_grant_mgmt_admin: build event failed: {e}");
                false
            }
        }
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

    pub(in crate::fabric::provider) fn parse_management_keys(
        &self,
    ) -> Option<nostr_sdk::prelude::Keys> {
        self.management_nsec
            .as_ref()
            .and_then(|n| nostr_sdk::prelude::Keys::parse(n).ok())
    }

    fn log_group_role_decision(project: &str, pubkey: &str, role: &str, reason: &str) {
        eprintln!(
            "[daemon] nip29-role-decision project={project} target={} role={role} reason={reason}",
            crate::util::pubkey_short(pubkey)
        );
    }

    /// Admin-add `pubkey_hex` to `project` as a plain member (not admin).
    pub async fn nip29_add_member(&self, project: &str, pubkey_hex: &str) -> bool {
        self.nip29_add_member_outcome(project, pubkey_hex)
            .await
            .is_applied()
    }

    pub(crate) async fn nip29_add_member_outcome(
        &self,
        project: &str,
        pubkey_hex: &str,
    ) -> GroupPublishOutcome {
        let Some(mgmt_keys) = self.parse_management_keys() else {
            return GroupPublishOutcome::Rejected;
        };
        Self::log_group_role_decision(project, pubkey_hex, "member", "add member");
        match crate::fabric::nip29::lifecycle::group_put_user(project, pubkey_hex) {
            Ok(b) => {
                self.publish_group_management_outcome(b, &mgmt_keys, "9000 put-user (session)")
                    .await
            }
            Err(e) => {
                tracing::error!(
                    group = project,
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
        let Some(mgmt_keys) = self.parse_management_keys() else {
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

    /// Admin-add `pubkey_hex` to `project` with the `admin` role.
    pub async fn nip29_add_admin(&self, project: &str, pubkey_hex: &str) -> bool {
        self.nip29_add_admin_outcome(project, pubkey_hex)
            .await
            .is_applied()
    }

    pub(crate) async fn nip29_add_admin_outcome(
        &self,
        project: &str,
        pubkey_hex: &str,
    ) -> GroupPublishOutcome {
        let Some(mgmt_keys) = self.parse_management_keys() else {
            return GroupPublishOutcome::Rejected;
        };
        Self::log_group_role_decision(project, pubkey_hex, "admin", "add admin");
        match crate::fabric::nip29::lifecycle::group_put_admin(project, pubkey_hex) {
            Ok(b) => {
                self.publish_group_management_outcome(b, &mgmt_keys, "9000 put-user (admin)")
                    .await
            }
            Err(e) => {
                tracing::error!(
                    group = project,
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
        let Some(mgmt_keys) = self.parse_management_keys() else {
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

    /// Admin-remove `pubkey_hex` from `project`.
    pub async fn nip29_remove_member(&self, project: &str, pubkey_hex: &str) -> bool {
        let Some(mgmt_keys) = self.parse_management_keys() else {
            return false;
        };
        eprintln!(
            "[daemon] nip29 remove-member h={project} p={}",
            crate::util::pubkey_short(pubkey_hex)
        );
        match crate::fabric::nip29::lifecycle::group_remove_user(project, pubkey_hex) {
            Ok(b) => {
                self.publish_group_management(b, &mgmt_keys, "9001 remove-user (session)")
                    .await
            }
            Err(e) => {
                tracing::error!(
                    group = project,
                    pubkey = pubkey_hex,
                    error = %format!("{e:#}"),
                    "nip29_remove_member: group_remove_user build failed — failing closed"
                );
                false
            }
        }
    }
}
