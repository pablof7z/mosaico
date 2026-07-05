use super::*;

impl DaemonState {
    pub(crate) async fn new_for_test() -> Arc<DaemonState> {
        let cfg = Config {
            whitelisted_pubkeys: Vec::new(),
            relays: Vec::new(),
            indexer_relay: String::new(),
            host: "test-host".into(),
            user_nsec: None,
            tenex_private_key: None,
            tmux_status_command: None,
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
            None,
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
            owners,
            hosted: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            subscribed_projects: Mutex::new(Vec::new()),
            subs: Mutex::new(crate::reconcile::SubscriptionReconciler::new().expect("subs")),
            status: Arc::new(Mutex::new(crate::reconcile::StatusReconciler::for_ttl(
                status_ttl_duration(),
            ))),
            turn_lifecycle: Mutex::new(crate::reconcile::TurnLifecycleReconciler::new()),
            cursor: Mutex::new(crate::reconcile::CursorReconciler::new()),
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
            last_status: Mutex::new(HashMap::new()),
            outbox_notify: Notify::new(),
            session_keys: Mutex::new(HashMap::new()),
            session_signers: Mutex::new(HashMap::new()),
            backend_pubkey: None,
        })
    }
}
