use crate::common::TestRelay;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn shared_relay_url() -> String {
    static RELAY: OnceLock<TestRelay> = OnceLock::new();
    RELAY.get_or_init(TestRelay::start).url.clone()
}

pub(crate) fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tenex-edge"))
}

pub(crate) struct Home {
    pub(crate) dir: tempfile::TempDir,
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
        Home { dir }
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

/// Run the real `tenex-edge` binary as a subprocess with the home's env — i.e.
/// exactly how the hooks invoke it. This is the ONLY path that exercises the
/// SYNCHRONOUS blocking client (`daemon::blocking`) + real CLI dispatch + the
/// actual stdout bytes the hooks parse.
pub(crate) fn run_cli(home: &Home, args: &[&str]) -> std::process::Output {
    std::process::Command::new(bin())
        .args(args)
        // Isolate from the invoking shell's tenex-edge env (a live claude/codex
        // shell exports these), so agent/session resolution is deterministic.
        .env_remove("TENEX_EDGE_AGENT")
        .env_remove("TENEX_EDGE_SESSION")
        .env("TENEX_EDGE_HOME", home.dir.path())
        .env("TENEX_CONFIG", home.dir.path().join("config.json"))
        .env("TENEX_EDGE_BIN", bin())
        .env("TENEX_EDGE_DAEMON_GRACE_S", "30")
        .output()
        .expect("run tenex-edge")
}

// Like run_cli, but pipes `stdin` to the child — used to drive the `hook`
// subcommand, which reads its harness payload from stdin (there are no longer
// any session/turn subcommands to call directly).
pub(crate) fn run_cli_stdin(home: &Home, args: &[&str], stdin: &str) -> std::process::Output {
    use std::io::Write as _;
    let mut child = std::process::Command::new(bin())
        .args(args)
        .env_remove("TENEX_EDGE_AGENT")
        .env_remove("TENEX_EDGE_SESSION")
        .env("TENEX_EDGE_HOME", home.dir.path())
        .env("TENEX_CONFIG", home.dir.path().join("config.json"))
        .env("TENEX_EDGE_BIN", bin())
        .env("TENEX_EDGE_DAEMON_GRACE_S", "30")
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
}
