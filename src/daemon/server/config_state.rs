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

    pub(crate) fn nmp(&self) -> Arc<crate::nmp_host::NmpHost> {
        self.nmp
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    /// The operator's whitelisted human pubkeys (config `whitelistedPubkeys`);
    /// classify a mention's sender as human vs agent for envelope presentation.
    pub(crate) fn whitelisted_pubkeys(&self) -> Vec<String> {
        self.owners()
    }

    /// The retained kind:0 profile feed driving `Store::get_profile`. The
    /// coverage refresh calls `set_members` on this `Arc`.
    pub(crate) fn profile_feed(&self) -> Arc<crate::nmp_host::ProfileFeed> {
        self.with_store(|store| store.profile_feed())
    }
}
