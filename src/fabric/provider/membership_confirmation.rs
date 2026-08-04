use super::Nip29Provider;
use crate::fabric::group_management::GroupMutationOutcome;
use crate::util::now_secs;

impl Nip29Provider {
    pub(crate) async fn grant_member_confirmed(
        &self,
        channel: &str,
        pubkey: &str,
    ) -> GroupMutationOutcome {
        self.confirm_role_grant(channel, pubkey, false).await
    }

    pub(crate) async fn grant_admin_confirmed(
        &self,
        channel: &str,
        pubkey: &str,
    ) -> GroupMutationOutcome {
        self.confirm_role_grant(channel, pubkey, true).await
    }

    pub(crate) async fn remove_member_confirmed(
        &self,
        channel: &str,
        pubkey: &str,
    ) -> GroupMutationOutcome {
        let mut last_readback_error = None;
        for attempt in 0..6u32 {
            let outcome = self.nip29_remove_member_outcome(channel, pubkey).await;
            // Read back from the cache the retained group-records observation
            // keeps current. Confirmation still rests on OBSERVED relay state:
            // the roster row only leaves the cache when a host publishes a
            // 39001/39002 that no longer names the subject.
            match self.with_store(|s| s.is_channel_member(channel, pubkey)) {
                Ok(false) => {
                    self.with_store(|s| {
                        if let Err(e) = s.remove_channel_member(channel, pubkey) {
                            tracing::error!(
                                channel,
                                pubkey,
                                error = %e,
                                "remove_member_confirmed: local mirror remove failed after confirmed relay removal"
                            );
                        }
                    });
                    return GroupMutationOutcome::Confirmed;
                }
                Ok(true) => {}
                Err(e) => {
                    last_readback_error = Some(format!("{e:#}"));
                    tracing::error!(
                        channel,
                        pubkey,
                        attempt,
                        error = %e,
                        "remove_member_confirmed: roster read-back failed; cannot confirm removal"
                    );
                }
            }
            if let crate::fabric::group_management::GroupPublishOutcome::Failed(error) = outcome {
                return GroupMutationOutcome::Failed(error);
            }
            tokio::time::sleep(std::time::Duration::from_millis(250 * (attempt as u64 + 1))).await;
        }
        GroupMutationOutcome::Unconfirmed {
            detail: last_readback_error
                .map(|error| format!("roster read-back failed: {error}"))
                .unwrap_or_else(|| "roster read-back still showed the member present".into()),
        }
    }

    async fn confirm_role_grant(
        &self,
        channel: &str,
        pubkey: &str,
        want_admin: bool,
    ) -> GroupMutationOutcome {
        let mut last_readback_error = None;
        for attempt in 0..6u32 {
            let outcome = if want_admin {
                self.nip29_add_admin_outcome(channel, pubkey).await
            } else {
                self.nip29_add_member_outcome(channel, pubkey).await
            };
            // Confirm ONLY on a relay state we actually OBSERVED. A read-back
            // failure must never be promoted to "grant confirmed".
            //
            // An admin grant is confirmed by the subject appearing on the
            // relay's kind:39001, NOT by that record carrying the literal role
            // string "admin". NIP-29's role position is a free-form label the
            // relay may leave empty; the admin list IS the grant. Requiring the
            // string meant a relay that wrote `["p", <pubkey>]` never confirmed
            // a grant it had in fact applied.
            match self.with_store(|s| {
                if want_admin {
                    s.is_channel_admin(channel, pubkey)
                } else {
                    s.is_channel_member(channel, pubkey)
                }
            }) {
                Ok(true) => {
                    let role = if want_admin { "admin" } else { "member" };
                    self.with_store(|s| {
                        if let Err(e) = s.upsert_channel_member(channel, pubkey, role, now_secs()) {
                            tracing::error!(
                                channel,
                                pubkey,
                                role,
                                error = %e,
                                "confirm_role_grant: local mirror write failed after confirmed relay grant"
                            );
                        }
                    });
                    return GroupMutationOutcome::Confirmed;
                }
                Ok(false) => {}
                Err(e) => {
                    last_readback_error = Some(format!("{e:#}"));
                    tracing::error!(
                        channel,
                        pubkey,
                        attempt,
                        error = %e,
                        "confirm_role_grant: roster read-back failed; cannot confirm grant"
                    );
                }
            }
            if let crate::fabric::group_management::GroupPublishOutcome::Failed(error) = outcome {
                return GroupMutationOutcome::Failed(error);
            }
            tokio::time::sleep(std::time::Duration::from_millis(250 * (attempt as u64 + 1))).await;
        }
        GroupMutationOutcome::Unconfirmed {
            detail: last_readback_error
                .map(|error| format!("roster read-back failed: {error}"))
                .unwrap_or_else(|| "roster read-back did not show the requested role".into()),
        }
    }
}
