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
        let backend_key = Keys::generate().secret_key().to_secret_hex();
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
        let transport = Arc::new(
            Transport::connect_with_indexer(&[], None, Keys::generate())
                .await
                .expect("offline transport connect"),
        );
        let store = Arc::new(Mutex::new(Store::open_memory().expect("in-memory store")));
        let nmp = Arc::new(
            crate::nmp_host::NmpHost::open(&[], None, None).expect("in-memory NMP engine"),
        );
        let provider = Arc::new(Nip29Provider::new(
            transport.clone(),
            nmp.clone(),
            store.clone(),
            Some(backend_key),
            None,
            Vec::new(),
        ));
        Arc::new(DaemonState {
            store,
            transport,
            provider,
            nmp,
            cfg,
            host,
            owners,
            agent_catalog: Mutex::new(crate::agent_catalog::AgentCatalog::default()),
            installed_harnesses: Mutex::new(installed_harnesses),
            hosted: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            subscribed_root_channels: Mutex::new(Vec::new()),
            subs: Mutex::new(crate::reconcile::SubscriptionReconciler::new()),
            subscription_sync: tokio::sync::Mutex::new(()),
            status: Arc::new(Mutex::new(crate::reconcile::StatusReconciler::for_ttl(
                status_ttl_duration(),
            ))),
            hook_contexts: Mutex::new(HashMap::new()),
            tail_tx: tokio::sync::broadcast::channel(512).0,
            open_clients: Mutex::new(0),
            shutdown: Notify::new(),
            peer_sessions: Mutex::new(HashMap::new()),
            seen_events: Mutex::new((
                std::collections::HashSet::new(),
                std::collections::VecDeque::new(),
            )),
            seen_profiles: Mutex::new(std::collections::HashSet::new()),
            warming: Mutex::new(std::collections::HashSet::new()),
            last_status: Mutex::new(HashMap::new()),
        })
    }
}
