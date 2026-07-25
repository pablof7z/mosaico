//! Keeping the local `kind:0` cache good enough for awareness to name people.
//!
//! Two entry points, two different problems. [`warm_profiles`] is opportunistic:
//! every inbound event names identities we may not have met, and fetching them
//! then means `who` never has to warm on the read path. [`refetch_missing_profiles`]
//! is corrective: the roster renderer withholds members it cannot name, and
//! withholding silently forever would be a lie of omission — so it reports what it
//! could not resolve and we go get it.
//!
//! Both are debounced, for different reasons. Warming collapses concurrent
//! duplicate deliveries of the same event; refetching spaces out retries for a
//! peer whose `kind:0` genuinely is not on the relays, so a permanently
//! unresolvable member surfaced on every turn cannot turn into a fetch loop.

use super::*;

/// Proactively fetch + cache the `kind:0` for any of `pubkeys` we do not already
/// have a name for. Called on every inbound event (a peer newly seen in a
/// 3900x/chat/status) and once at startup for the identities we already know
/// (owners, hosted agents). Known identities are filtered out cheaply and
/// synchronously — they never spawn a task or touch the network — and concurrent
/// duplicate deliveries of the same event collapse to ONE in-flight fetch per
/// pubkey via the `warming` guard. `who` therefore never has to warm: the cache
/// is populated as pubkeys are observed, and it renders names from the cache.
pub(in crate::daemon::server) fn warm_profiles(state: &Arc<DaemonState>, pubkeys: Vec<String>) {
    let to_fetch = claim_pubkeys_to_warm(state, pubkeys);
    if to_fetch.is_empty() {
        return;
    }
    let st = state.clone();
    tokio::spawn(async move {
        for pk in &to_fetch {
            let _ = crate::profile::resolve_name(&st, pk).await;
        }
        // Release the in-flight claims; a fetch that failed (offline relay) is thus
        // retried the next time the pubkey is observed rather than being wedged.
        let mut guard = st.dedup.warming_profiles.lock().unwrap();
        for pk in &to_fetch {
            guard.remove(pk);
        }
    });
}

/// The synchronous half of [`warm_profiles`]: reduce `pubkeys` to the ones worth a
/// relay fetch and claim them in the in-flight `warming` set. A pubkey is dropped
/// when it is empty, already has a cached name, or is already being fetched — so a
/// known identity never hits the network and duplicate deliveries never stack up.
fn claim_pubkeys_to_warm(state: &Arc<DaemonState>, pubkeys: Vec<String>) -> Vec<String> {
    // A cache miss (no row, or a row with no resolved name) is the only reason to
    // hit the relay; everything already named is skipped.
    let missing = state.with_store(|s| {
        pubkeys
            .into_iter()
            .filter(|pk| !pk.is_empty())
            .filter(|pk| {
                s.get_profile(pk)
                    .ok()
                    .flatten()
                    .map(|p| p.name.is_empty())
                    .unwrap_or(true)
            })
            .collect::<Vec<_>>()
    });
    // Collapse concurrent duplicates: claim each pubkey; a fetch already in flight
    // for it keeps ownership until it completes.
    let mut guard = state.dedup.warming_profiles.lock().unwrap();
    missing
        .into_iter()
        .filter(|pk| guard.insert(pk.clone()))
        .collect()
}

/// Minimum spacing between kind:0 refetch attempts for the same pubkey.
const PROFILE_REFETCH_DEBOUNCE_SECS: u64 = 90;

/// Force a kind:0 (re)fetch for roster members awareness could not name,
/// bypassing [`crate::profile::resolve_name`]'s cache TTL — the whole point is
/// that the cached answer is the problem. Unlike [`warm_profiles`] this is driven
/// by the render path, so it must assume the same unresolvable peer arrives every
/// single turn; the debounce is what keeps that from becoming a relay hammer.
pub(in crate::daemon::server) fn refetch_missing_profiles(
    state: &Arc<DaemonState>,
    pubkeys: Vec<String>,
) {
    let to_fetch = claim_profiles_to_refetch(state, pubkeys, now_secs());
    if to_fetch.is_empty() {
        return;
    }
    let st = state.clone();
    tokio::spawn(async move {
        let provider = st.fabric_provider();
        for pk in &to_fetch {
            let _ = provider.fetch_and_cache_profile_name(pk, now_secs()).await;
        }
    });
}

/// The synchronous half of [`refetch_missing_profiles`]: reduce `pubkeys` to the
/// ones outside the debounce window and stamp them as attempted. Attempts older
/// than the window are forgotten on the way through, so the ledger stays bounded
/// by the number of pubkeys seen in one window rather than growing forever.
fn claim_profiles_to_refetch(
    state: &Arc<DaemonState>,
    pubkeys: Vec<String>,
    now: u64,
) -> Vec<String> {
    if pubkeys.is_empty() {
        return Vec::new();
    }
    let mut guard = state.dedup.profile_refetch_attempts.lock().unwrap();
    guard.retain(|_, last| now.saturating_sub(*last) < PROFILE_REFETCH_DEBOUNCE_SECS);
    pubkeys
        .into_iter()
        .filter(|pk| !pk.is_empty())
        .filter(|pk| match guard.get(pk) {
            Some(last) if now.saturating_sub(*last) < PROFILE_REFETCH_DEBOUNCE_SECS => false,
            _ => {
                guard.insert(pk.clone(), now);
                true
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
