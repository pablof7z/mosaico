use super::*;

impl DaemonState {
    pub(crate) async fn new_for_test() -> Arc<DaemonState> {
        Self::new_for_test_with_started_at(0).await
    }

    pub(crate) async fn new_for_test_with_started_at(started_at: u64) -> Arc<DaemonState> {
        let backend_key = Keys::generate().secret_key().to_secret_hex();
        let cfg = Config {
            whitelisted_pubkeys: Vec::new(),
            relays: Vec::new(),
            indexer_relay: String::new(),
            host: "test-host".into(),
            user_nsec: None,
            tenex_private_key: Some(backend_key.clone()),
            per_session_rooms: false,
        };
        let host = cfg.host.clone();
        let owners = cfg.whitelisted_pubkeys.clone();
        let transport = Arc::new(
            Transport::connect(&[], Keys::generate())
                .await
                .expect("offline transport connect"),
        );
        let store = Arc::new(Mutex::new(Store::open_memory().expect("in-memory store")));
        let provider = Arc::new(Nip29Provider::new(
            transport.clone(),
            store.clone(),
            Some(backend_key),
            None,
            Vec::new(),
            &cfg.relays,
        ));
        Arc::new(DaemonState {
            store,
            transport,
            provider,
            cfg,
            host,
            started_at,
            owners,
            hosted: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            subscribed_root_channels: Mutex::new(Vec::new()),
            subs: Mutex::new(crate::reconcile::SubscriptionReconciler::new().expect("subs")),
            status: Arc::new(Mutex::new(crate::reconcile::StatusReconciler::for_ttl(
                status_ttl_duration(),
            ))),
            delivery: Mutex::new(crate::reconcile::DeliveryReconciler::new()),
            turn_lifecycle: Mutex::new(crate::reconcile::TurnLifecycleReconciler::new()),
            cursor: Mutex::new(crate::reconcile::CursorReconciler::new()),
            session_start: Mutex::new(crate::reconcile::SessionStartReconciler::new()),
            session_watch: Mutex::new(crate::reconcile::Reconciler::new().expect("session_watch")),
            outbox: Arc::new(Mutex::new(crate::reconcile::OutboxReconciler::new())),
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
            outbox_notify: Notify::new(),
            session_keys: Mutex::new(HashMap::new()),
        })
    }
}
