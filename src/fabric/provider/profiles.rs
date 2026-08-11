use super::Nip29Provider;
use crate::fabric::RawEnvelope;
use std::time::Duration;

const PROFILE_FETCH_TIMEOUT: Duration = Duration::from_secs(4);

impl Nip29Provider {
    pub(crate) async fn fetch_and_cache_profile_name(
        &self,
        pubkey: &str,
        _now: u64,
    ) -> Option<String> {
        nostr::PublicKey::from_hex(pubkey).ok()?;
        let filter = crate::nmp_host::read::filter(&[0], &[pubkey.to_string()], &[]).ok()?;
        // Profile display is intentionally cache-tolerant: a signed kind:0 is
        // self-authenticating, so a timed-out live acquisition may still yield
        // a useful cached name. The read result remains typed until this policy
        // decision; doctor and provisioning require stronger evidence.
        let read = self
            .nmp
            .fetch_profiles(filter, 1, PROFILE_FETCH_TIMEOUT)
            .await
            .ok()?;
        let event = read
            .rows
            .into_iter()
            .map(|row| row.event)
            .max_by_key(|event| event.created_at)?;
        self.with_store(|store| {
            self.materialize(&RawEnvelope::Nostr(event), store);
        });
        self.with_store(|store| {
            store
                .get_profile(pubkey)
                .ok()
                .flatten()
                .map(|profile| profile.name)
                .filter(|name| !name.is_empty())
        })
    }
}
