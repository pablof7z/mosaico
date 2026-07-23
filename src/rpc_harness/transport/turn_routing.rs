//! Per-child app-server turn correlation and launch-time history.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use super::{RpcError, RpcHandle};
use crate::rpc_harness::protocol::RpcErrorObject;

#[derive(Debug)]
pub(crate) enum TurnSignal {
    Completed(serde_json::Value),
    Reconcile,
}

type TurnWaiters = Arc<Mutex<HashMap<String, mpsc::UnboundedSender<TurnSignal>>>>;

/// Cancellation-safe registration for one app-server thread observer.
pub(crate) struct TurnObserver {
    key: String,
    sender: mpsc::UnboundedSender<TurnSignal>,
    receiver: mpsc::UnboundedReceiver<TurnSignal>,
    waiters: TurnWaiters,
}

impl TurnObserver {
    pub(crate) async fn recv(&mut self) -> Option<TurnSignal> {
        self.receiver.recv().await
    }
}

impl Drop for TurnObserver {
    fn drop(&mut self) {
        let mut waiters = self.waiters.lock().unwrap();
        let registered_here = waiters
            .get(&self.key)
            .is_some_and(|sender| sender.same_channel(&self.sender));
        if registered_here {
            waiters.remove(&self.key);
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct AppServerRouting {
    waiters: TurnWaiters,
    baselines: Arc<Mutex<HashMap<String, HashSet<String>>>>,
}

impl AppServerRouting {
    pub(crate) fn register(&self, key: &str) -> Result<TurnObserver, RpcError> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let mut waiters = self.waiters.lock().unwrap();
        if waiters.contains_key(key) {
            return Err(RpcError::Protocol(RpcErrorObject {
                code: -1,
                message: format!("app-server thread {key} already has an active turn observer"),
                data: None,
            }));
        }
        waiters.insert(key.to_string(), sender.clone());
        drop(waiters);
        Ok(TurnObserver {
            key: key.to_string(),
            sender,
            receiver,
            waiters: self.waiters.clone(),
        })
    }

    pub(crate) fn signal(&self, key: &str, signal: TurnSignal) {
        if let Some(sender) = self.waiters.lock().unwrap().get(key).cloned() {
            let _ = sender.send(signal);
        }
    }

    pub(crate) fn clear_waiters(&self) {
        self.waiters.lock().unwrap().clear();
    }

    pub(crate) fn record_baseline(&self, key: &str, turn_ids: HashSet<String>) {
        self.baselines
            .lock()
            .unwrap()
            .insert(key.to_string(), turn_ids);
    }

    pub(crate) fn take_baseline(&self, key: &str) -> Option<HashSet<String>> {
        self.baselines.lock().unwrap().remove(key)
    }
}

impl RpcHandle {
    /// Register the sole active turn observer for one app-server thread.
    pub(crate) fn register_turn_waiter(&self, key: &str) -> Result<TurnObserver, RpcError> {
        self.app_server_routing.register(key)
    }

    /// Preserve launch-time history until the first delivery on this child.
    pub(crate) fn record_turn_baseline(&self, key: &str, turn_ids: HashSet<String>) {
        self.app_server_routing.record_baseline(key, turn_ids);
    }

    pub(crate) fn take_turn_baseline(&self, key: &str) -> Option<HashSet<String>> {
        self.app_server_routing.take_baseline(key)
    }
}
