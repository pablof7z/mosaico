//! Strict in-process NIP-42 relay for the Mosaico/NMP consumer capstone.

use std::collections::{BTreeMap, BTreeSet};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use nostr::{Event, EventId, Filter, PublicKey, RelayUrl};

mod protocol;
mod server;
use server::run_relay;

#[derive(Clone, Debug, Default)]
pub(super) struct AuthRelayObservation {
    pub(super) auth_events: Vec<Event>,
    pub(super) invalid_auth: Vec<String>,
    pub(super) ordinary_events: Vec<Event>,
    pub(super) authenticated_reqs: Vec<(PublicKey, Vec<Filter>)>,
    pub(super) pre_auth_reqs: usize,
    pub(super) pre_auth_events: usize,
}

#[derive(Default)]
struct AuthRelayState {
    events: BTreeMap<EventId, Event>,
    observation: AuthRelayObservation,
    connections: usize,
}

pub(super) struct AuthRequiredRelay {
    url: RelayUrl,
    state: Arc<Mutex<AuthRelayState>>,
    shutdown: Arc<AtomicBool>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl AuthRequiredRelay {
    pub(super) fn spawn(
        allowed_pubkeys: impl IntoIterator<Item = PublicKey>,
        seed: impl IntoIterator<Item = Event>,
    ) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind AUTH relay");
        listener
            .set_nonblocking(true)
            .expect("make AUTH listener cancellable");
        let port = listener.local_addr().expect("AUTH relay address").port();
        let url = RelayUrl::parse(&format!("ws://127.0.0.1:{port}")).expect("parse AUTH relay URL");
        let allowed = allowed_pubkeys.into_iter().collect::<BTreeSet<_>>();
        assert!(!allowed.is_empty(), "AUTH harness requires an identity");
        let state = Arc::new(Mutex::new(AuthRelayState {
            events: seed.into_iter().map(|event| (event.id, event)).collect(),
            ..AuthRelayState::default()
        }));
        let shutdown = Arc::new(AtomicBool::new(false));

        let relay_url = url.clone();
        let relay_state = Arc::clone(&state);
        let relay_shutdown = Arc::clone(&shutdown);
        let join = thread::spawn(move || {
            run_relay(listener, relay_url, allowed, relay_state, relay_shutdown);
        });
        Self {
            url,
            state,
            shutdown,
            join: Mutex::new(Some(join)),
        }
    }

    pub(super) fn url(&self) -> String {
        self.url.to_string()
    }

    pub(super) fn observation(&self) -> AuthRelayObservation {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .observation
            .clone()
    }

    pub(super) fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(join) = self
            .join
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
        {
            if let Err(panic) = join.join() {
                if !thread::panicking() {
                    std::panic::resume_unwind(panic);
                }
            }
        }
    }
}

impl Drop for AuthRequiredRelay {
    fn drop(&mut self) {
        self.shutdown();
    }
}
