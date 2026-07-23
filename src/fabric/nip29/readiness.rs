use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::Mutex as AsyncMutex;

/// How long a channel stays verified without a re-check (5 min). Lease renewals
/// fire every 30 s; only the first one per window hits the relay.
pub(crate) const READY_TTL_SECS: u64 = 300;

/// Context for a channel readiness check.
pub struct ChannelCtx<'a> {
    /// NIP-29 group h-tag the publish targets.
    pub channel: &'a str,
    /// Pubkey (hex) that must be at least a member before the publish proceeds.
    pub expect_member: &'a str,
    /// Soft parent hint: the h-tag of the parent group to ensure first when
    /// `channel` doesn't yet exist on the relay. Ignored when the channel is
    /// already present (the relay's stored hierarchy wins). `None` creates the
    /// channel as a top-level group.
    pub parent_hint: Option<&'a str>,
    /// Caller's intended display NAME, used ONLY when this readiness check has to
    /// CREATE the (sub)group — it rides on the published kind:9002 metadata so the
    /// relay's authored kind:39000 carries it. NEVER written to `relay_channels`
    /// locally: that cache is materialized solely from observed relay events. A
    /// root group always names itself after its slug, so `None` is correct there.
    pub name: Option<&'a str>,
    /// Whether this check should also repair whitelisted human admin grants.
    pub repair_whitelisted_admins: bool,
}

/// Outcome of a readiness check.
#[derive(Debug)]
pub enum ChannelGate {
    /// Channel was already ready; nothing touched.
    Ready,
    /// Channel was missing, incomplete, or needed repairs; corrected.
    Repaired,
    /// Could not fully verify or repair the channel (relay unreachable, no
    /// management key, roster grant not confirmed, etc.). The channel is NOT
    /// verified — callers MUST treat this as a failure and refuse to publish into
    /// the channel (no longer fail-open: publishing into an unverified channel
    /// would risk writing against a group whose existence/membership we could not
    /// confirm against relay truth).
    Degraded,
}

/// Resolve a soft host-local parent hint without overriding relay truth.
/// `Some("")` is an observed root channel and therefore suppresses fallback;
/// only absent relay metadata may use the pending host context.
pub(crate) fn effective_parent_hint(
    relay_parent: Option<String>,
    pending_parent: Option<&str>,
    channel: &str,
) -> Option<String> {
    match relay_parent {
        Some(parent) => (!parent.is_empty()).then_some(parent),
        None => pending_parent
            .filter(|parent| !parent.is_empty() && *parent != channel)
            .map(str::to_string),
    }
}

struct ChannelSlot {
    verified_at: Option<Instant>,
}

/// In-process TTL'd cache tracking which channels are known-ready.
///
/// The cache key is `(channel, expect_member)` so that different agents
/// publishing to the same channel each get an independent readiness slot.
/// Without this, the first agent to mark a channel ready would suppress
/// provisioning for subsequent agents that may not yet be members.
pub struct ChannelReadiness {
    inner: Mutex<HashMap<(String, String), ChannelSlot>>,
    inflight_by_channel: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
}

impl Default for ChannelReadiness {
    fn default() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            inflight_by_channel: Mutex::new(HashMap::new()),
        }
    }
}

impl ChannelReadiness {
    /// Returns `(is_ready_right_now, inflight_lock_for_this_channel_and_member)`.
    /// The caller acquires the inflight lock before doing any relay I/O, then
    /// calls this again after acquiring to double-check (another task may have
    /// repaired it while we waited for the lock).
    pub(crate) fn check(&self, channel: &str, expect_member: &str) -> (bool, Arc<AsyncMutex<()>>) {
        let ready = {
            let mut map = self.inner.lock().expect("readiness map poisoned");
            let key = (channel.to_string(), expect_member.to_string());
            let slot = map
                .entry(key)
                .or_insert_with(|| ChannelSlot { verified_at: None });
            slot.verified_at
                .map(|t| t.elapsed().as_secs() < READY_TTL_SECS)
                .unwrap_or(false)
        };
        let inflight = self
            .inflight_by_channel
            .lock()
            .expect("readiness inflight map poisoned")
            .entry(channel.to_string())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        (ready, inflight)
    }

    pub(crate) fn mark_ready(&self, channel: &str, expect_member: &str) {
        let mut map = self.inner.lock().expect("readiness map poisoned");
        let key = (channel.to_string(), expect_member.to_string());
        let slot = map
            .entry(key)
            .or_insert_with(|| ChannelSlot { verified_at: None });
        slot.verified_at = Some(Instant::now());
    }

    /// Invalidate a channel+member pair (e.g. after an observed relay-side
    /// roster change), forcing a re-verify on the next publish.
    pub fn invalidate(&self, channel: &str, expect_member: &str) {
        let mut map = self.inner.lock().expect("readiness map poisoned");
        let key = (channel.to_string(), expect_member.to_string());
        if let Some(slot) = map.get_mut(&key) {
            slot.verified_at = None;
        }
    }

    /// Invalidate every cached member readiness slot for a channel. Relay-authored
    /// admin/member snapshots replace the roster, so any per-member readiness
    /// proof for that channel must be re-checked on the next publish.
    pub(crate) fn invalidate_channel(&self, channel: &str) {
        let mut map = self.inner.lock().expect("readiness map poisoned");
        for ((slot_channel, _), slot) in map.iter_mut() {
            if slot_channel == channel {
                slot.verified_at = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_invalidation_clears_all_members_for_that_channel_only() {
        let readiness = ChannelReadiness::default();
        readiness.mark_ready("chan-a", "alice");
        readiness.mark_ready("chan-a", "bob");
        readiness.mark_ready("chan-b", "alice");

        readiness.invalidate_channel("chan-a");

        assert!(!readiness.check("chan-a", "alice").0);
        assert!(!readiness.check("chan-a", "bob").0);
        assert!(readiness.check("chan-b", "alice").0);
    }

    #[test]
    fn relay_parent_state_precedes_pending_host_context() {
        assert_eq!(
            effective_parent_hint(Some("relay-parent".into()), Some("host-parent"), "room"),
            Some("relay-parent".into())
        );
        assert_eq!(
            effective_parent_hint(Some(String::new()), Some("host-parent"), "room"),
            None,
            "an observed relay root must suppress the fallback"
        );
        assert_eq!(
            effective_parent_hint(None, Some("host-parent"), "room"),
            Some("host-parent".into())
        );
    }

    /// The invite RPC's `ensure_backend_admin` wraps its readiness future in a
    /// bounded `tokio::time::timeout`, mapping an elapsed timeout to
    /// `ChannelGate::Degraded` (then a bail) so an unreachable relay can never
    /// wedge the invite call — and the client connection with it — forever.
    ///
    /// The full function needs an `Arc<DaemonState>` + a live/fake relay provider,
    /// so it is exercised end-to-end only by an integration test. This isolates
    /// the timeout-wrapping contract it depends on: a never-ready readiness future
    /// must ELAPSE into `Degraded` rather than hang, and `Degraded` must produce a
    /// bounded error rather than a stall.
    #[tokio::test]
    async fn timeout_wrapping_maps_stalled_readiness_to_degraded_bail() {
        use std::time::Duration;

        // Mirror the production wrapper exactly: run a readiness future under a
        // bounded timeout, map an elapsed timeout to Degraded, then bail on it.
        async fn ensure_ready_bounded(
            timeout: Duration,
            ready: impl std::future::Future<Output = ChannelGate>,
        ) -> anyhow::Result<()> {
            let gate = match tokio::time::timeout(timeout, ready).await {
                Ok(gate) => gate,
                Err(_) => ChannelGate::Degraded,
            };
            if matches!(gate, ChannelGate::Degraded) {
                anyhow::bail!("channel is not ready for remote invite");
            }
            Ok(())
        }

        // A readiness probe that never resolves (a wedged relay). Bounded by the
        // timeout, this returns promptly with an error instead of hanging.
        let stalled = std::future::pending::<ChannelGate>();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            ensure_ready_bounded(Duration::from_millis(10), stalled),
        )
        .await
        .expect("the bounded wrapper must not hang past its own timeout");

        let err = result.expect_err("a stalled readiness probe must surface an error");
        assert!(
            err.to_string().contains("not ready for remote invite"),
            "unexpected error: {err}"
        );

        // And a Ready gate that resolves in time passes through cleanly.
        let ok =
            ensure_ready_bounded(Duration::from_millis(10), async { ChannelGate::Ready }).await;
        assert!(
            ok.is_ok(),
            "a ready channel must not be treated as degraded"
        );
    }
}
