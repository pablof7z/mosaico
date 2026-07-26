use super::*;

impl StatusReconciler {
    /// Reassert the complete current status after relay membership admission.
    /// This is intentionally not deduplicated: an earlier publication may have
    /// failed while the signer was not yet a confirmed channel member.
    pub fn reassert(
        &mut self,
        pubkey: &str,
        generation: u64,
        projection: PresenceProjection,
        now: u64,
    ) -> StatusOutcome {
        let Some(state) = self.owned_mut(pubkey, generation) else {
            return self.empty_outcome(pubkey);
        };
        let before = command_of(pubkey, state);
        state.snapshot.projection = projection;
        state.live = true;
        let after = command_of(pubkey, state);
        let effects = if before.channels.is_empty() && after.channels.is_empty() {
            Vec::new()
        } else if !before.channels.is_empty() && after.channels.is_empty() {
            vec![StatusEffect::Expire {
                status: status_build::to_status(&before, self.ttl_secs, now, true),
            }]
        } else {
            vec![StatusEffect::Publish {
                status: status_build::to_status(&after, self.ttl_secs, now, false),
                reason: PublishReason::Admitted,
            }]
        };
        self.outcome(pubkey, effects)
    }
}
