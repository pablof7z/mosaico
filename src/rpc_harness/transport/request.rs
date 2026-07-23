//! Correlated JSON-RPC requests over a live child transport.

use std::sync::atomic::Ordering;

use tokio::sync::oneshot;

use super::{PendingMap, RpcError, RpcHandle};

struct PendingGuard {
    pending: PendingMap,
    id: i64,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.pending.lock().unwrap().remove(&self.id);
    }
}

impl RpcHandle {
    fn next_id(&self) -> i64 {
        self.ids.fetch_add(1, Ordering::Relaxed)
    }

    /// Send a request and await its correlated response.
    pub async fn request(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RpcError> {
        let id = self.next_id();
        let (sender, receiver) = oneshot::channel();
        // `alive` changes before the reader drains this same map on EOF. Thus a
        // request is either inserted for that drain or rejected as already dead.
        {
            let mut pending = self.pending.lock().unwrap();
            if !self.alive.load(Ordering::Relaxed) {
                return Err(RpcError::ChildExited);
            }
            pending.insert(id, sender);
        }
        // Cancellation, including request_timeout, must remove the correlation.
        let _pending_guard = PendingGuard {
            pending: self.pending.clone(),
            id,
        };
        let line = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        })
        .to_string();
        if self.writer.send(line).await.is_err() {
            return Err(RpcError::ChildExited);
        }
        match receiver.await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(RpcError::ChildExited),
        }
    }

    /// Send a request with a timeout.
    pub async fn request_timeout(
        &self,
        method: &str,
        params: serde_json::Value,
        duration: std::time::Duration,
    ) -> Result<serde_json::Value, RpcError> {
        match tokio::time::timeout(duration, self.request(method, params)).await {
            Ok(result) => result,
            Err(_) => Err(RpcError::Timeout),
        }
    }

    /// Fire-and-forget notification (no id, no await).
    pub async fn notify(&self, method: &str, params: serde_json::Value) {
        let line = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        })
        .to_string();
        let _ = self.writer.send(line).await;
    }

    #[cfg(test)]
    pub(crate) fn pending_request_count(&self) -> usize {
        self.pending.lock().unwrap().len()
    }
}
