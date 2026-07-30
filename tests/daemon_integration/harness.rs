use crate::common::TestRelay;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[path = "harness/daemon.rs"]
mod daemon;
pub(crate) use daemon::stop_daemon;
#[path = "harness/cli.rs"]
mod cli;
pub(crate) use cli::{
    run_cli, run_cli_stdin, run_cli_stdin_with_env, run_cli_stdin_with_env_in_dir,
    run_cli_with_env, run_cli_with_env_in_dir,
};
#[path = "harness/launch.rs"]
mod launch;
pub(crate) use launch::{
    configure_pty_agent, configure_pty_agent_with_args, install_test_harness_shim,
};
#[path = "harness/pty_guard.rs"]
mod pty_guard;
pub(crate) use pty_guard::PtyProcessGuard;
#[path = "harness/reconcile_witness.rs"]
mod reconcile_witness;
pub(crate) use reconcile_witness::{daemon_log_boundary, wait_for_reconciled_session_engine};
#[path = "harness/relay_witness.rs"]
mod relay_witness;
pub(crate) use relay_witness::{publish_addressed_chat, wait_for_exact_relay_groups};
#[path = "harness/wedge_relay.rs"]
mod wedge_relay;
pub(crate) use wedge_relay::WedgeRelay;

pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn hook_session_start(
    mut params: serde_json::Value,
    observed_harness: &str,
) -> serde_json::Value {
    let object = params.as_object_mut().expect("session-start params object");
    object.insert(
        "observed_harness".into(),
        observed_harness.to_string().into(),
    );
    object.insert(
        "claimed_harness".into(),
        observed_harness.to_string().into(),
    );
    object.insert("endpoint_provenance".into(), "hook".into());
    params
}

pub(crate) fn shared_relay_url() -> String {
    static RELAY: OnceLock<TestRelay> = OnceLock::new();
    RELAY.get_or_init(TestRelay::start).url.clone()
}

/// A shared NIP-29 relay for tests that own groups / mint subgroups
/// (nak can't do NIP-29). Shared only within one test thread, so relay state
/// cannot leak between integration tests.
pub(crate) fn shared_nip29_relay_url() -> String {
    thread_local! {
        static RELAY: RefCell<Option<TestRelay>> = const { RefCell::new(None) };
    }
    RELAY.with(|relay| {
        let mut relay = relay.borrow_mut();
        relay
            .get_or_insert_with(TestRelay::start_nip29_relay)
            .url
            .clone()
    })
}

pub(crate) fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mosaico"))
}

pub(crate) struct Home {
    pub(crate) dir: tempfile::TempDir,
    original_home: Option<std::ffi::OsString>,
}

impl Drop for Home {
    fn drop(&mut self) {
        stop_daemon(self);
        unsafe {
            match self.original_home.take() {
                Some(home) => std::env::set_var("HOME", home),
                None => std::env::remove_var("HOME"),
            }
        }
    }
}

impl Home {
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let original_home = std::env::var_os("HOME");
        unsafe { std::env::set_var("HOME", dir.path()) };
        install_test_harness_shim(dir.path());
        std::env::set_var("MOSAICO_HOME", dir.path());
        let cfg = dir.path().join("config.json");
        let body = serde_json::json!({
            "whitelistedPubkeys": [],
            "backendName": "test-host",
            "relays": [shared_relay_url()],
        });
        std::fs::write(&cfg, serde_json::to_string(&body).unwrap()).unwrap();
        std::env::set_var("MOSAICO_CONFIG", &cfg);
        std::env::set_var("MOSAICO_DAEMON_GRACE_S", "30");
        std::env::set_var("MOSAICO_BIN", bin());
        // Register /tmp as a channel so hooks (which all send cwd=/tmp) find a
        // resolvable channel. Without this, the new "refuse to start without a
        // known channel" gate silently exits 0 and the tests see no session.
        let workspace_map = serde_json::json!({ "tmp": "/tmp" });
        std::fs::write(
            dir.path().join("workspaces.json"),
            serde_json::to_string(&workspace_map).unwrap(),
        )
        .unwrap();
        Home { dir, original_home }
    }

    pub(crate) fn with_wedged_relay(relay_url: &str) -> Self {
        let home = Self::new();
        let cfg = home.dir.path().join("config.json");
        let body = serde_json::json!({
            "whitelistedPubkeys": [],
            "backendName": "test-host",
            "relays": [relay_url],
            "indexerRelay": relay_url,
        });
        std::fs::write(&cfg, serde_json::to_string(&body).unwrap()).unwrap();
        home
    }
    /// Rewrite the config to include a backend signing key (`mosaicoPrivateKey`).
    /// Needed by tests that start multiple CONCURRENT same-agent sessions in one
    /// channel: with per-session rooms off (the default) they share the channel
    /// channel and thus the durable signer slot, so the second session derives a
    /// transient "second-personality" key — which requires a backend key.
    pub(crate) fn with_backend_key(self) -> Self {
        let cfg = self.dir.path().join("config.json");
        let body = serde_json::json!({
            "whitelistedPubkeys": [],
            "backendName": "test-host",
            "relays": [shared_nip29_relay_url()],
            "indexerRelay": shared_nip29_relay_url(),
            "mosaicoPrivateKey": "b53809614e9c41b923dd5546e438e48e9bcbee04b9c7c50bae0b085954e03422",
        });
        std::fs::write(&cfg, serde_json::to_string(&body).unwrap()).unwrap();
        self
    }
    pub(crate) fn store_path(&self) -> PathBuf {
        self.dir.path().join("state.db")
    }
    pub(crate) fn sock(&self) -> PathBuf {
        self.dir.path().join("daemon.sock")
    }
}

pub(crate) fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().unwrap()
}

/// Poll `pred` until it returns true or `timeout` elapses. Per-session rooms are
/// minted on the relay in a background task (session start does not block on the
/// relay), so tests must wait for relay-backed state (e.g. room membership)
/// before asserting on it or publishing into the room.
pub(crate) fn wait_until(timeout: Duration, mut pred: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if pred() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Chat events materialized in a channel, oldest-first. Chat lives verbatim in
/// `relay_events`; row `.content` is the message body and `.pubkey` the author.
pub(crate) fn chat_in_channel(
    store: &mosaico::state::Store,
    channel_h: &str,
) -> Vec<mosaico::state::RelayEvent> {
    store.chat_for_channel(channel_h, 0, u32::MAX).unwrap()
}

/// The selected ordinal signer pubkey bound to a session, or `None` when no
/// session identity row has been materialized yet.
pub(crate) fn session_identity_pubkey(
    store: &mosaico::state::Store,
    pubkey: &str,
) -> Option<String> {
    store.session_identity(pubkey).unwrap().map(|i| i.pubkey)
}

/// Resolve a harness-owned native session id through its typed locator.
pub(crate) fn pubkey_for_harness_session(
    store: &mosaico::state::Store,
    harness: &str,
    harness_session: &str,
) -> Option<String> {
    store
        .resolve_pubkey_by_locator(harness, "native_resume", harness_session)
        .unwrap()
}

pub(crate) fn session_for_harness_session(
    store: &mosaico::state::Store,
    harness: &str,
    harness_session: &str,
) -> mosaico::state::Session {
    let pubkey = pubkey_for_harness_session(store, harness, harness_session)
        .expect("harness session locator");
    store.get_session(&pubkey).unwrap().expect("session row")
}

pub(crate) fn session_routes(store: &mosaico::state::Store, pubkey: &str) -> Vec<String> {
    store
        .list_session_routes(pubkey)
        .expect("session routes")
        .into_iter()
        .map(|(channel_h, _)| channel_h)
        .collect()
}

pub(crate) fn only_session_route(store: &mosaico::state::Store, pubkey: &str) -> String {
    let routes = session_routes(store, pubkey);
    assert_eq!(routes.len(), 1, "expected exactly one session route");
    routes.into_iter().next().unwrap()
}

/// The PTY supervisor id bound to a session via its `pty_session` alias, if any.
/// Replaces the removed `get_session_endpoint(session, "pty")`.
pub(crate) fn pty_session_for_session(
    store: &mosaico::state::Store,
    pubkey: &str,
) -> Option<String> {
    store
        .locators_for_pubkey(pubkey)
        .unwrap()
        .into_iter()
        .find(|locator| locator.locator_kind == "pty")
        .map(|locator| locator.locator_value)
}
