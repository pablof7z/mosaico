use super::*;

impl DaemonState {
    /// Capture the complete relay-facing runtime generation for one operation.
    pub(crate) fn snapshot(&self) -> Arc<RuntimeSnapshot> {
        self.runtime_snapshot
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    pub(super) fn install_snapshot(&self, next: Arc<RuntimeSnapshot>) -> Arc<RuntimeSnapshot> {
        self.store
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .bind_nmp_views(next.nmp.views_handle());
        let mut slot = self
            .runtime_snapshot
            .write()
            .unwrap_or_else(|poison| poison.into_inner());
        std::mem::replace(&mut *slot, next)
    }

    pub(crate) fn host(&self) -> String {
        self.snapshot().config.host.clone()
    }

    pub(crate) fn owners(&self) -> Vec<String> {
        self.snapshot().config.whitelisted_pubkeys.clone()
    }

    /// The operator's whitelisted human pubkeys (config `whitelistedPubkeys`).
    pub(crate) fn whitelisted_pubkeys(&self) -> Vec<String> {
        self.owners()
    }
}
