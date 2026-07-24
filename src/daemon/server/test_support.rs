use super::*;

impl DaemonState {
    pub(crate) async fn new_for_test() -> Arc<DaemonState> {
        Self::new_for_test_with(Vec::new()).await
    }

    pub(crate) async fn new_for_test_with_whitelisted(
        whitelisted_pubkeys: Vec<String>,
    ) -> Arc<DaemonState> {
        Self::new_for_test_with(whitelisted_pubkeys).await
    }

    async fn new_for_test_with(whitelisted_pubkeys: Vec<String>) -> Arc<DaemonState> {
        let backend_keys = Keys::generate();
        let backend_key = backend_keys.secret_key().to_secret_hex();
        let installed_harnesses = crate::harness::HarnessesConfig::load()
            .unwrap_or_default()
            .bundles
            .into_values()
            .map(|bundle| bundle.harness)
            .fold(Vec::new(), |mut harnesses, harness| {
                if !harnesses.contains(&harness) {
                    harnesses.push(harness);
                }
                harnesses
            });
        let cfg = Config {
            whitelisted_pubkeys,
            relays: Vec::new(),
            indexer_relay: String::new(),
            host: "test-host".into(),
            user_nsec: None,
            mosaico_private_key: Some(backend_key.clone()),
            per_session_rooms: false,
        };
        let host = cfg.host.clone();
        let owners = cfg.whitelisted_pubkeys.clone();
        let store = Arc::new(Mutex::new(Store::open_memory().expect("in-memory store")));
        let nmp = Arc::new(
            crate::nmp_host::NmpHost::open(&[], None, None, &backend_keys)
                .expect("in-memory NMP engine"),
        );
        let provider = Arc::new(Nip29Provider::new(
            nmp.clone(),
            store.clone(),
            Some(backend_key),
            None,
            Vec::new(),
        ));
        let presence_publisher =
            crate::presence_publisher::PresencePublisher::spawn(provider.clone(), store.clone());
        let catalog = CatalogState::new();
        *catalog.harnesses.lock().unwrap() = installed_harnesses;
        Arc::new(DaemonState {
            store,
            provider,
            nmp,
            cfg,
            host,
            owners,
            agent_config: AgentConfigState::new(),
            catalog,
            runtime: SessionRuntimeState::new(),
            subscriptions: SubscriptionState::new(),
            reconcilers: ReconcilerState::new(
                crate::reconcile::StatusReconciler::for_ttl(presence_lease_ttl()),
                presence_publisher,
            ),
            connections: ConnectionState::new(),
            dedup: DedupState::new(),
            standing_sync: tokio::sync::Mutex::new(()),
            mcp_actor_sync: tokio::sync::Mutex::new(()),
        })
    }
}
