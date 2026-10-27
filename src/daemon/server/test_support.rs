use super::*;

impl DaemonState {
    pub(crate) async fn new_for_test() -> Arc<DaemonState> {
        Self::new_for_test_with(Vec::new(), "wss://relay.invalid".to_string()).await
    }

    pub(crate) async fn new_for_test_with_whitelisted(
        whitelisted_pubkeys: Vec<String>,
    ) -> Arc<DaemonState> {
        Self::new_for_test_with(whitelisted_pubkeys, "wss://relay.invalid".to_string()).await
    }

    async fn new_for_test_with(
        whitelisted_pubkeys: Vec<String>,
        relay: String,
    ) -> Arc<DaemonState> {
        let backend_keys = Keys::generate();
        let backend_key = backend_keys.secret_key().to_secret_hex();
        let attachment_receive_directory = std::env::temp_dir().join(format!(
            "mosaico-test-attachments-{}",
            backend_keys.public_key().to_hex()
        ));
        let installed_harnesses = crate::config::detect_available_harnesses().unwrap_or_default();
        let cfg = Config {
            whitelisted_pubkeys: whitelisted_pubkeys.clone(),
            relay: relay.clone(),
            host: "test-host".into(),
            user_nsec: None,
            mosaico_private_key: Some(backend_key.clone()),
            per_session_rooms: false,
            cross_project_boundary: crate::config::CrossProjectBoundary::default(),
            attachment_receive_directory,
        };
        let nostr = Arc::new(crate::nostr_runtime::NostrRuntime::new(&relay).unwrap());
        let view = Arc::new(crate::fabric::nip29::view::Nip29ViewStore::default());
        let mut store = Store::open_memory().expect("in-memory store");
        store.bind_nip29_view(view.clone());
        let store = Arc::new(Mutex::new(store));
        let provider = Arc::new(Nip29Provider::new(
            nostr.clone(),
            view.clone(),
            store.clone(),
            backend_keys.public_key(),
            None,
            whitelisted_pubkeys,
        ));
        let provider = Arc::new(std::sync::RwLock::new(provider));
        let nostr = Arc::new(std::sync::RwLock::new(nostr));
        let nip29_view = Arc::new(std::sync::RwLock::new(view));
        let presence_publisher =
            crate::presence_publisher::PresencePublisher::spawn(provider.clone(), store.clone());
        let catalog = CatalogState::new();
        *catalog.harnesses.lock().unwrap() = installed_harnesses;
        Arc::new(DaemonState {
            store,
            provider,
            nostr,
            nip29_view,
            backend_keys: std::sync::RwLock::new(backend_keys),
            cfg: std::sync::RwLock::new(cfg),
            config_reload: tokio::sync::Mutex::new(()),
            agent_config: AgentConfigState::new(),
            catalog,
            profile_refresh: ProfileRefreshState::new(),
            runtime: SessionRuntimeState::new(),
            subscriptions: SubscriptionState::new(),
            reconcilers: ReconcilerState::new(
                crate::reconcile::StatusReconciler::for_ttl(presence_lease_ttl()),
                presence_publisher,
            ),
            connections: ConnectionState::new(),
            dedup: DedupState::new(),
            standing_sync: tokio::sync::Mutex::new(()),
            leaving: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            mcp_actor_sync: tokio::sync::Mutex::new(()),
        })
    }
}
