use crate::common::TestRelay;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[path = "harness/daemon.rs"]
mod daemon;
pub(crate) use daemon::stop_daemon;
#[path = "harness/launch.rs"]
mod launch;
pub(crate) use launch::{
    configure_pty_agent, configure_pty_agent_with_args, install_test_harness_shim,
};
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
}

impl Drop for Home {
    fn drop(&mut self) {
        stop_daemon(self);
    }
}

impl Home {
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
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
        Home { dir }
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

/// Run the real `mosaico` binary as a subprocess with the home's env — i.e.
/// exactly how the hooks invoke it. This is the ONLY path that exercises the
/// SYNCHRONOUS blocking client (`daemon::blocking`) + real CLI dispatch + the
/// actual stdout bytes the hooks parse.
pub(crate) fn run_cli(home: &Home, args: &[&str]) -> std::process::Output {
    cli_command(home, args).output().expect("run mosaico")
}

pub(crate) fn run_cli_with_env(
    home: &Home,
    args: &[&str],
    env: &[(&str, &str)],
) -> std::process::Output {
    let mut cmd = cli_command(home, args);
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd.output().expect("run mosaico")
}

pub(crate) fn run_cli_with_env_in_dir(
    home: &Home,
    args: &[&str],
    env: &[(&str, &str)],
    cwd: &std::path::Path,
) -> std::process::Output {
    let mut cmd = cli_command(home, args);
    cmd.current_dir(cwd);
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd.output().expect("run mosaico")
}

fn cli_command(home: &Home, args: &[&str]) -> std::process::Command {
    let mut cmd = std::process::Command::new(bin());
    cmd.args(args)
        // Isolate from the invoking shell's mosaico env (a live claude/codex
        // shell exports these), so session pubkey derivation is deterministic.
        .env_remove("MOSAICO_AGENT")
        .env_remove("MOSAICO_PUBKEY")
        .env_remove("MOSAICO_PTY_SESSION")
        .env_remove("MOSAICO_PTY_SOCKET")
        .env_remove("MOSAICO_CHANNEL")
        .env_remove("MOSAICO_EPHEMERAL")
        .env("MOSAICO_HOME", home.dir.path())
        .env("MOSAICO_CONFIG", home.dir.path().join("config.json"))
        .env("MOSAICO_BIN", bin())
        .env("MOSAICO_DAEMON_GRACE_S", "30");
    cmd
}

// Like run_cli, but pipes `stdin` to the child — used to drive the `hook`
// subcommand, which reads its harness payload from stdin (there are no longer
// any session/turn subcommands to call directly).
pub(crate) fn run_cli_stdin(home: &Home, args: &[&str], stdin: &str) -> std::process::Output {
    run_cli_stdin_with_env(home, args, stdin, &[])
}

pub(crate) fn run_cli_stdin_with_env(
    home: &Home,
    args: &[&str],
    stdin: &str,
    env: &[(&str, &str)],
) -> std::process::Output {
    use std::io::Write as _;
    let mut cmd = cli_command(home, args);
    for (key, value) in env {
        cmd.env(key, value);
    }
    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn mosaico");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("run mosaico")
}

pub(crate) fn run_cli_stdin_with_env_in_dir(
    home: &Home,
    args: &[&str],
    stdin: &str,
    env: &[(&str, &str)],
    cwd: &std::path::Path,
) -> std::process::Output {
    use std::io::Write as _;
    let mut cmd = cli_command(home, args);
    cmd.current_dir(cwd);
    for (key, value) in env {
        cmd.env(key, value);
    }
    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn mosaico");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("run mosaico")
}

/// Chat (kind:9/11) events materialized in a channel, oldest-first. Replaces the
/// removed `Store::list_chat_messages`/`peek_chat` channel reader — chat now lives
/// verbatim in `relay_events`, read back via `chat_for_channel`. Row fields map:
/// `.body` → `.content`, `.from_pubkey` → `.pubkey`.
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
