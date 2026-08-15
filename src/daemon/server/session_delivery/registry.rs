use super::DaemonState;
use crate::state::Session;
use crate::util::now_secs;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub(super) const LIVE_GRACE_SECS: u64 = 75;

#[derive(Default)]
pub(in crate::daemon::server) struct ActiveExtensionDeliveryRegistry {
    next_lease: u64,
    waiting: HashMap<(String, u64), u32>,
    live_until: HashMap<(String, u64), u64>,
    leases: HashMap<String, DeliveryLease>,
}

pub(super) struct DeliveryLease {
    pub(super) pubkey: String,
    pub(super) generation: u64,
    pub(super) event_ids: Vec<String>,
    pub(super) reminder_turn: Option<u64>,
}

impl ActiveExtensionDeliveryRegistry {
    pub(super) fn touch(&mut self, rec: &Session, now: u64) -> bool {
        let was_live = self.live_for(rec, now);
        self.live_until.insert(
            (rec.pubkey.clone(), rec.runtime_generation),
            now.saturating_add(LIVE_GRACE_SECS),
        );
        !was_live
    }

    pub(super) fn begin_wait(&mut self, rec: &Session) {
        *self
            .waiting
            .entry((rec.pubkey.clone(), rec.runtime_generation))
            .or_default() += 1;
    }

    fn end_wait(&mut self, rec: &Session) {
        let key = (rec.pubkey.clone(), rec.runtime_generation);
        let Some(count) = self.waiting.get_mut(&key) else {
            return;
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            self.waiting.remove(&key);
        }
    }

    pub(super) fn live_for(&self, rec: &Session, now: u64) -> bool {
        self.live_until
            .get(&(rec.pubkey.clone(), rec.runtime_generation))
            .is_some_and(|until| *until >= now)
    }

    pub(super) fn insert_lease(
        &mut self,
        rec: &Session,
        event_ids: Vec<String>,
        reminder_turn: Option<u64>,
    ) -> String {
        self.next_lease = self.next_lease.saturating_add(1).max(1);
        let lease_id = format!("pi-{}-{}", rec.runtime_generation, self.next_lease);
        self.leases.insert(
            lease_id.clone(),
            DeliveryLease {
                pubkey: rec.pubkey.clone(),
                generation: rec.runtime_generation,
                event_ids,
                reminder_turn,
            },
        );
        lease_id
    }

    pub(super) fn take_lease(&mut self, rec: &Session, lease_id: &str) -> Option<DeliveryLease> {
        let lease = self.leases.get(lease_id)?;
        if lease.pubkey != rec.pubkey || lease.generation != rec.runtime_generation {
            return None;
        }
        self.leases.remove(lease_id)
    }
}

pub(super) struct ExtensionWaitGuard {
    registry: Arc<Mutex<ActiveExtensionDeliveryRegistry>>,
    rec: Session,
}

impl ExtensionWaitGuard {
    pub(super) fn new(registry: Arc<Mutex<ActiveExtensionDeliveryRegistry>>, rec: Session) -> Self {
        Self { registry, rec }
    }
}

impl Drop for ExtensionWaitGuard {
    fn drop(&mut self) {
        self.registry
            .lock()
            .expect("extension-delivery mutex poisoned")
            .end_wait(&self.rec);
    }
}

pub(crate) fn extension_delivery_live(state: &DaemonState, rec: &Session) -> bool {
    state
        .runtime
        .extension_delivery
        .lock()
        .expect("extension-delivery mutex poisoned")
        .live_for(rec, now_secs())
}
