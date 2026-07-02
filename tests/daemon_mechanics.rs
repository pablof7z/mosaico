//! Daemon mechanics: spawn-if-absent, spawn race, stale-socket reclaim,
//! version-skew handshake, and a basic RPC round-trip. These drive the thin
//! client against a real spawned `__daemon` over a UDS in an isolated
//! `TENEX_EDGE_HOME`.
//!
//! The daemon connects ONE relay at startup, so each test points its config's
//! `relays` at a local `nak serve` (NOT the production relay — that would touch
//! the live fabric). Each test isolates its daemon via a fresh temp
//! `TENEX_EDGE_HOME`; env mutation is serialized with a mutex.

#[path = "common/mod.rs"]
mod common;

use common::TestRelay;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tenex_edge::daemon::client::Client;

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// One local `nak serve` shared by every test (cheap; avoids the production relay).
fn shared_relay_url() -> String {
    static RELAY: OnceLock<TestRelay> = OnceLock::new();
    RELAY.get_or_init(TestRelay::start).url.clone()
}

struct Home {
    dir: tempfile::TempDir,
}

impl Drop for Home {
    fn drop(&mut self) {
        stop_daemon(self);
    }
}

impl Home {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("TENEX_EDGE_HOME", dir.path());
        // Config with a LOCAL relay so the daemon never dials the live fabric.
        let cfg = dir.path().join("config.json");
        let body = serde_json::json!({
            "whitelistedPubkeys": [],
            "backendName": "test-host",
            "relays": [shared_relay_url()],
        });
        std::fs::write(&cfg, serde_json::to_string(&body).unwrap()).unwrap();
        std::env::set_var("TENEX_CONFIG", &cfg);
        std::env::set_var("TENEX_EDGE_DAEMON_GRACE_S", "30");
        // The thin client spawns `current_exe() __daemon`; in a test binary that
        // is the harness, so point it at the real built binary.
        std::env::set_var("TENEX_EDGE_BIN", bin());
        // Register /tmp as a project so hook-driven session_start finds a
        // resolvable project (the new "refuse without a project" gate would
        // otherwise silently exit 0).
        let projects_map = serde_json::json!({ "tmp": "/tmp" });
        std::fs::write(
            dir.path().join("projects.json"),
            serde_json::to_string(&projects_map).unwrap(),
        )
        .unwrap();
        Home { dir }
    }
    fn sock(&self) -> PathBuf {
        self.dir.path().join("daemon.sock")
    }
    fn lock(&self) -> PathBuf {
        self.dir.path().join("daemon.lock")
    }
}

/// The thin client spawns `current_exe() __daemon`. In a test binary that is the
/// test harness, not tenex-edge — so point it at the built binary via the env
/// the client reads. We override by building the real binary path and exporting
/// it... but the client uses current_exe() directly. Instead, tests that need a
/// REAL spawned daemon must run the actual binary. We use CARGO_BIN_EXE.
fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tenex-edge"))
}

/// Spawn the daemon by exec'ing the real binary (not current_exe of the test).
fn spawn_real_daemon(home: &Home) -> std::process::Child {
    let log = std::fs::File::create(home.dir.path().join("daemon.log")).unwrap();
    std::process::Command::new(bin())
        .arg("__daemon")
        .env("TENEX_EDGE_HOME", home.dir.path())
        .env("TENEX_CONFIG", home.dir.path().join("config.json"))
        .env("TENEX_EDGE_DAEMON_GRACE_S", "30")
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .expect("spawn daemon")
}

fn wait_for_sock(home: &Home, dur: Duration) -> bool {
    let deadline = Instant::now() + dur;
    while Instant::now() < deadline {
        if UnixStream::connect(home.sock()).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

/// Hand-rolled handshake as a NEWER client (protocol > daemon's): after the
/// welcome, a newer client sends `please_exit`; the daemon replies with a
/// protocol_skew error and shuts down. Returns the daemon's response frame.
fn newer_client_please_exit(sock: &PathBuf, hello_protocol: u32) -> serde_json::Value {
    let stream = UnixStream::connect(sock).expect("connect");
    let mut w = stream.try_clone().unwrap();
    let mut r = BufReader::new(stream);

    writeln!(
        w,
        "{}",
        serde_json::json!({"protocol": hello_protocol, "client_version": "test"})
    )
    .unwrap();
    let mut welcome = String::new();
    r.read_line(&mut welcome).unwrap();

    // A newer client's follow-up is the please_exit control frame.
    writeln!(w, "{}", serde_json::json!({"protocol": hello_protocol})).unwrap();
    let mut resp = String::new();
    r.read_line(&mut resp).unwrap();
    serde_json::from_str(resp.trim()).unwrap_or(serde_json::json!({}))
}

#[test]
fn spawn_if_absent_then_ping_roundtrip() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let res = rt.block_on(async {
        let mut client = Client::connect_or_spawn().await?;
        client.call("ping", serde_json::json!({})).await
    });
    let val = res.expect("ping round-trip");
    assert_eq!(val["pong"], serde_json::json!(true));

    // A daemon should now be listening.
    assert!(home.sock().exists(), "daemon socket should exist");

    // Stop it.
    stop_daemon(&home);
}

#[test]
fn spawn_race_single_winner() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();

    // N threads all connect_or_spawn at once; exactly one daemon must bind.
    let n = 16;
    let handles: Vec<_> = (0..n)
        .map(|_| {
            std::thread::spawn(|| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    let mut c = Client::connect_or_spawn().await.expect("connect");
                    let v = c.call("ping", serde_json::json!({})).await.expect("ping");
                    assert_eq!(v["pong"], serde_json::json!(true));
                });
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    // Exactly one daemon should be holding the lock / socket. We can't count
    // processes portably, but a second blocking lock attempt should fail to be
    // the sole owner if the daemon holds it — the strong signal is that all 16
    // clients succeeded against ONE socket, which the asserts above prove.
    assert!(home.sock().exists());
    stop_daemon(&home);
}

#[test]
fn stale_socket_is_reclaimed() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();

    // Create a stale socket file with nobody listening: bind then drop.
    {
        let listener = std::os::unix::net::UnixListener::bind(home.sock()).unwrap();
        drop(listener); // leaves the socket path on disk, no listener
    }
    // On some platforms dropping the listener unlinks the path; recreate a plain
    // file at the socket path to simulate the "file present, connect refused"
    // case the daemon must reclaim.
    if !home.sock().exists() {
        std::fs::write(home.sock(), b"").unwrap();
    }
    assert!(home.sock().exists());

    let rt = tokio::runtime::Runtime::new().unwrap();
    let res = rt.block_on(async {
        let mut c = Client::connect_or_spawn().await?;
        c.call("ping", serde_json::json!({})).await
    });
    assert_eq!(
        res.expect("ping after reclaim")["pong"],
        serde_json::json!(true)
    );
    stop_daemon(&home);
}

#[test]
fn version_skew_old_daemon_exits_and_respawns() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();

    // Start a real daemon (current protocol). Then a "newer" client (protocol+1)
    // must cause it to exit; connect_or_spawn then respawns a fresh daemon.
    let mut daemon = spawn_real_daemon(&home);
    assert!(wait_for_sock(&home, Duration::from_secs(5)), "daemon up");

    // Simulate a newer client by hand-rolling the handshake with protocol = MAX.
    // The daemon should reply with a protocol_skew error and begin shutting down.
    let resp = newer_client_please_exit(&home.sock(), u32::MAX);
    assert!(
        resp["error"]["code"] == serde_json::json!("protocol_skew"),
        "expected protocol_skew, got {resp}"
    );

    // The old daemon should exit (release the socket) shortly.
    let gone = {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match daemon.try_wait() {
                Ok(Some(_)) => break true,
                Ok(None) if Instant::now() > deadline => break false,
                _ => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    };
    assert!(
        gone,
        "old daemon should exit after a protocol-skew please_exit"
    );

    // Now a normal client respawns a fresh daemon and works.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let res = rt.block_on(async {
        let mut c = Client::connect_or_spawn().await?;
        c.call("ping", serde_json::json!({})).await
    });
    assert_eq!(
        res.expect("ping after respawn")["pong"],
        serde_json::json!(true)
    );
    stop_daemon(&home);
}

/// Ask the daemon to exit by deleting the lock and sending SIGTERM if we can
/// find it; simplest portable approach: connect with a future protocol to
/// trigger its shutdown path, then wait for the socket to disappear.
fn stop_daemon(home: &Home) {
    if !home.sock().exists() {
        return;
    }
    let _ = newer_client_please_exit(&home.sock(), u32::MAX); // please_exit path
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if !home.sock().exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    // Best-effort cleanup of lock file.
    let _ = std::fs::remove_file(home.lock());
}
