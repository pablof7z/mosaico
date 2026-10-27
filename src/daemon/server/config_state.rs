use super::*;

impl DaemonState {
    pub(crate) fn config(&self) -> Config {
        self.cfg
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    pub(crate) fn host(&self) -> String {
        self.config().host
    }

    pub(crate) fn owners(&self) -> Vec<String> {
        self.config().whitelisted_pubkeys
    }

    pub(crate) fn provider(&self) -> Arc<Nip29Provider> {
        self.provider
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    pub(crate) fn nostr(&self) -> Arc<crate::nostr_runtime::NostrRuntime> {
        self.nostr
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    pub(crate) fn nip29_view(&self) -> Arc<crate::fabric::nip29::view::Nip29ViewStore> {
        self.nip29_view
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    /// The operator's whitelisted human pubkeys (config `whitelistedPubkeys`);
    /// classify a mention's sender as human vs agent for envelope presentation.
    pub(crate) fn whitelisted_pubkeys(&self) -> Vec<String> {
        self.owners()
    }

    #[cfg(feature = "stress-harness")]
    pub(crate) fn stress_nostr_snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "active_demands": self.nostr().active_demand_count(),
            "profile_authors": self.nostr().profile_author_count(),
        })
    }
}
