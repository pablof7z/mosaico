use crate::common::TestRelay;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());

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
    PathBuf::from(env!("CARGO_BIN_EXE_tenex-edge"))
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
        std::env::set_var("TENEX_EDGE_HOME", dir.path());
        let cfg = dir.path().join("config.json");
        let body = serde_json::json!({
            "whitelistedPubkeys": [],
            "backendName": "test-host",
            "relays": [shared_relay_url()],
        });
        std::fs::write(&cfg, serde_json::to_string(&body).unwrap()).unwrap();
        std::env::set_var("TENEX_CONFIG", &cfg);
        std::env::set_var("TENEX_EDGE_DAEMON_GRACE_S", "30");
        std::env::set_var("TENEX_EDGE_BIN", bin());
        // Register /tmp as a project so hooks (which all send cwd=/tmp) find a
        // resolvable project. Without this, the new "refuse to start without a
        // known project" gate silently exits 0 and the tests see no session.
        let projects_map = serde_json::json!({ "tmp": "/tmp" });
        std::fs::write(
            dir.path().join("projects.json"),
            serde_json::to_string(&projects_map).unwrap(),
        )
        .unwrap();
        Home { dir }
    }
    /// Rewrite the config to include a backend signing key (`tenexPrivateKey`).
    /// Needed by tests that start multiple CONCURRENT same-agent sessions in one
    /// project: with per-session rooms off (the default) they share the project
    /// channel and thus the durable signer slot, so the second session derives a
    /// transient "second-personality" key — which requires a backend key.
    pub(crate) fn with_backend_key(self) -> Self {
        let cfg = self.dir.path().join("config.json");
        let body = serde_json::json!({
            "whitelistedPubkeys": [],
            "backendName": "test-host",
            "relays": [shared_nip29_relay_url()],
            "indexerRelay": shared_nip29_relay_url(),
            "tenexPrivateKey": "b53809614e9c41b923dd5546e438e48e9bcbee04b9c7c50bae0b085954e03422",
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

/// Run the real `tenex-edge` binary as a subprocess with the home's env — i.e.
/// exactly how the hooks invoke it. This is the ONLY path that exercises the
/// SYNCHRONOUS blocking client (`daemon::blocking`) + real CLI dispatch + the
/// actual stdout bytes the hooks parse.
pub(crate) fn run_cli(home: &Home, args: &[&str]) -> std::process::Output {
    cli_command(home, args).output().expect("run tenex-edge")
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
    cmd.output().expect("run tenex-edge")
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
    cmd.output().expect("run tenex-edge")
}

fn cli_command(home: &Home, args: &[&str]) -> std::process::Command {
    let mut cmd = std::process::Command::new(bin());
    cmd.args(args)
        // Isolate from the invoking shell's tenex-edge env (a live claude/codex
        // shell exports these), so agent/session resolution is deterministic.
        .env_remove("TENEX_EDGE_AGENT")
        .env_remove("TENEX_EDGE_AGENT_FALLBACK")
        .env_remove("TENEX_EDGE_SESSION")
        .env_remove("TENEX_EDGE_PTY_SESSION")
        .env_remove("TENEX_EDGE_PTY_SOCKET")
        .env_remove("TENEX_EDGE_CHANNEL")
        .env_remove("TENEX_EDGE_EPHEMERAL")
        .env("TENEX_EDGE_HOME", home.dir.path())
        .env("TENEX_CONFIG", home.dir.path().join("config.json"))
        .env("TENEX_EDGE_BIN", bin())
        .env("TENEX_EDGE_DAEMON_GRACE_S", "30");
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
        .expect("spawn tenex-edge");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("run tenex-edge")
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
        .expect("spawn tenex-edge");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("run tenex-edge")
}

/// Chat (kind:9/11) events materialized in a channel, oldest-first. Replaces the
/// removed `Store::list_chat_messages`/`peek_chat` channel reader — chat now lives
/// verbatim in `relay_events`, read back via `chat_for_channel`. Row fields map:
/// `.body` → `.content`, `.from_pubkey` → `.pubkey`.
pub(crate) fn chat_in_channel(
    store: &tenex_edge::state::Store,
    channel_h: &str,
) -> Vec<tenex_edge::state::RelayEvent> {
    store.chat_for_channel(channel_h, 0, u32::MAX).unwrap()
}

/// The selected ordinal signer pubkey bound to a session, or `None` when no
/// session identity row has been materialized yet.
pub(crate) fn session_identity_pubkey(
    store: &tenex_edge::state::Store,
    session_id: &str,
) -> Option<String> {
    store
        .identity_for_session(session_id)
        .unwrap()
        .map(|i| i.pubkey)
}

/// The PTY supervisor id bound to a session via its `pty_session` alias, if any.
/// Replaces the removed `get_session_endpoint(session, "pty")`.
pub(crate) fn pty_session_for_session(
    store: &tenex_edge::state::Store,
    session_id: &str,
) -> Option<String> {
    store
        .aliases_for_session(session_id)
        .unwrap()
        .into_iter()
        .find(|a| a.external_id_kind == "pty_session")
        .map(|a| a.external_id)
}

/// Stop the daemon by sending the version-skew please_exit and waiting for the
/// socket to disappear (keeps tests from leaking daemons).
pub(crate) fn stop_daemon(home: &Home) {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    if let Ok(stream) = UnixStream::connect(home.sock()) {
        let mut w = stream.try_clone().unwrap();
        let mut r = BufReader::new(stream);
        let _ = writeln!(
            w,
            "{}",
            serde_json::json!({"protocol": u32::MAX, "client_version": "t"})
        );
        let mut welcome = String::new();
        let _ = r.read_line(&mut welcome);
        let _ = writeln!(w, "{}", serde_json::json!({"protocol": u32::MAX}));
        let mut resp = String::new();
        let _ = r.read_line(&mut resp);
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && home.sock().exists() {
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = std::fs::remove_file(home.dir.path().join("daemon.lock"));
}
