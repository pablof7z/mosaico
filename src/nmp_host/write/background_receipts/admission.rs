use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

pub(super) struct AdmissionState {
    pub(super) pending: usize,
    pub(super) closed: bool,
}

pub(super) struct Admission {
    pub(super) state: Mutex<AdmissionState>,
    pub(super) changed: Condvar,
    pub(super) capacity: usize,
}

impl Admission {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new(AdmissionState {
                pending: 0,
                closed: false,
            }),
            changed: Condvar::new(),
            capacity,
        }
    }
}

pub(super) struct ReceiptSlot(pub(super) Arc<Admission>);

impl Drop for ReceiptSlot {
    fn drop(&mut self) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.pending = state.pending.saturating_sub(1);
        self.0.changed.notify_all();
    }
}

pub(in crate::nmp_host::write) struct BackgroundReceiptPermit {
    pub(super) admission: Arc<Admission>,
    pub(super) unassigned: usize,
    pub(super) deadline: Instant,
}

impl BackgroundReceiptPermit {
    pub(super) fn take_slot(&mut self) -> ReceiptSlot {
        debug_assert!(self.unassigned > 0);
        self.unassigned -= 1;
        ReceiptSlot(Arc::clone(&self.admission))
    }
}

impl Drop for BackgroundReceiptPermit {
    fn drop(&mut self) {
        for _ in 0..self.unassigned {
            drop(ReceiptSlot(Arc::clone(&self.admission)));
        }
    }
}
