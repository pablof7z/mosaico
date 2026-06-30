use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::Mutex as AsyncMutex;

/// How long a channel stays verified without a re-check (5 min). Heartbeats
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

struct ChannelSlot {
    verified_at: Option<Instant>,
    /// Per-channel single-flight: concurrent readiness checks coalesce on this.
    inflight: Arc<AsyncMutex<()>>,
}

/// In-process TTL'd cache tracking which channels are known-ready.
///
/// The cache key is `(channel, expect_member)` so that different agents
/// publishing to the same channel each get an independent readiness slot.
/// Without this, the first agent to mark a channel ready would suppress
/// provisioning for subsequent agents that may not yet be members.
pub struct ChannelReadiness {
    inner: Mutex<HashMap<(String, String), ChannelSlot>>,
}

impl Default for ChannelReadiness {
    fn default() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

impl ChannelReadiness {
    /// Returns `(is_ready_right_now, inflight_lock_for_this_channel_and_member)`.
    /// The caller acquires the inflight lock before doing any relay I/O, then
    /// calls this again after acquiring to double-check (another task may have
    /// repaired it while we waited for the lock).
    pub(crate) fn check(&self, channel: &str, expect_member: &str) -> (bool, Arc<AsyncMutex<()>>) {
        let mut map = self.inner.lock().expect("readiness map poisoned");
        let key = (channel.to_string(), expect_member.to_string());
        let slot = map.entry(key).or_insert_with(|| ChannelSlot {
            verified_at: None,
            inflight: Arc::new(AsyncMutex::new(())),
        });
        let ready = slot
            .verified_at
            .map(|t| t.elapsed().as_secs() < READY_TTL_SECS)
            .unwrap_or(false);
        (ready, slot.inflight.clone())
    }

    pub(crate) fn mark_ready(&self, channel: &str, expect_member: &str) {
        let mut map = self.inner.lock().expect("readiness map poisoned");
        let key = (channel.to_string(), expect_member.to_string());
        let slot = map.entry(key).or_insert_with(|| ChannelSlot {
            verified_at: None,
            inflight: Arc::new(AsyncMutex::new(())),
        });
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
}
