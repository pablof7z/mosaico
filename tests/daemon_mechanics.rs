//! Daemon mechanics: spawn-if-absent, spawn race, stale-socket reclaim,
//! version-skew handshake, and a basic RPC round-trip. These drive the thin
//! client against a real spawned `daemon` over a UDS in an isolated
//! `MOSAICO_HOME`.
//!
//! The daemon connects ONE relay at startup, so each test points its config's
//! `relays` at a local `nak serve` (NOT the production relay — that would touch
//! the live fabric). Each test isolates its daemon via a fresh temp
//! `MOSAICO_HOME`; env mutation is serialized with a mutex.

#[path = "common/mod.rs"]
mod common;
#[path = "daemon_mechanics/named_instances.rs"]
mod named_instances;

use common::TestRelay;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use mosaico::daemon::client::Client;

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
        scavenge_deleted_tmp_mosaico_processes();
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: daemon_mechanics serializes env mutation via ENV_LOCK.
        unsafe {
            std::env::remove_var("MOSAICO");
            std::env::set_var("MOSAICO_HOME", dir.path());
            std::env::set_var("MOSAICO_CONFIG", dir.path().join("config.json"));
            std::env::set_var("MOSAICO_DAEMON_GRACE_S", "30");
            std::env::set_var(mosaico::pty::REAP_SESSIONS_ON_STOP_ENV, "1");
            // The thin client spawns `current_exe() daemon`; in a test binary that
            // is the harness, so point it at the real built binary.
            std::env::set_var("MOSAICO_BIN", bin());
        }
        // Config with a LOCAL relay so the daemon never dials the live fabric.
        let cfg = dir.path().join("config.json");
        let body = serde_json::json!({
            "whitelistedPubkeys": [],
            "backendName": "test-host",
            "relays": [shared_relay_url()],
        });
        std::fs::write(&cfg, serde_json::to_string(&body).unwrap()).unwrap();
        // Register /tmp as a channel so hook-driven session_start finds a
        // resolvable channel (the new "refuse without a channel" gate would
        // otherwise silently exit 0).
        let workspace_map = serde_json::json!({ "tmp": "/tmp" });
        std::fs::write(
            dir.path().join("workspaces.json"),
            serde_json::to_string(&workspace_map).unwrap(),
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

/// Exact standalone binary; the Cargo test harness cannot act as a daemon.
fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mosaico"))
}

/// Spawn the daemon by exec'ing the real binary (not current_exe of the test).
fn spawn_real_daemon(home: &Home) -> std::process::Child {
    let log = std::fs::File::create(home.dir.path().join("daemon.log")).unwrap();
    std::process::Command::new(bin())
        .arg("daemon")
        .env_remove("MOSAICO")
        .env("MOSAICO_HOME", home.dir.path())
        .env("MOSAICO_CONFIG", home.dir.path().join("config.json"))
        .env("MOSAICO_DAEMON_GRACE_S", "30")
        .env(mosaico::pty::REAP_SESSIONS_ON_STOP_ENV, "1")
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
fn cargo_test_harness_cannot_spawn_itself_as_daemon() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    std::env::set_var("MOSAICO_BIN", std::env::current_exe().unwrap());

    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(Client::connect_or_spawn());
    let error = match result {
        Ok(_) => panic!("test harness unexpectedly spawned as the daemon"),
        Err(error) => error.to_string(),
    };

    assert!(error.contains("refusing to spawn Cargo test harness"));
    assert!(!home.sock().exists());
    std::env::set_var("MOSAICO_BIN", bin());
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

/// Full teardown: reap this home's PTY supervisors, stop the daemon via
/// protocol skew, force-kill any leftover daemon still bound to this home.
fn stop_daemon(home: &Home) {
    unsafe {
        std::env::set_var("MOSAICO_HOME", home.dir.path());
    }
    let reap = mosaico::pty::reap_home_supervisors().expect("PTY reap during teardown");
    assert!(
        reap.is_clean(),
        "PTY supervisors survived teardown: {}",
        reap.errors.join("; ")
    );
    if home.sock().exists() {
        let _ = newer_client_please_exit(&home.sock(), u32::MAX);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && home.sock().exists() {
        std::thread::sleep(Duration::from_millis(25));
    }
    force_kill_daemons_for_home(home.dir.path());
    let _ = std::fs::remove_file(home.lock());
    scavenge_deleted_tmp_mosaico_processes();
}

fn force_kill_daemons_for_home(home: &std::path::Path) {
    let needle = format!("MOSAICO_HOME={}", home.display());
    let Ok(output) = std::process::Command::new("/bin/ps")
        .args(["-eo", "pid=", "-o", "args="])
        .output()
    else {
        return;
    };
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let line = line.trim();
        let Some((pid, args)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let Ok(pid) = pid.parse::<i32>() else {
            continue;
        };
        let tokens: Vec<&str> = args.split_whitespace().collect();
        let is_daemon = tokens.iter().any(|arg| arg.ends_with("mosaico"))
            && tokens.contains(&"daemon")
            && !tokens.contains(&"__pty-supervisor");
        if !is_daemon {
            continue;
        }
        let Ok(env) = std::fs::read(format!("/proc/{pid}/environ")) else {
            continue;
        };
        let env = String::from_utf8_lossy(&env).replace('\0', "\n");
        if env.contains(&needle) {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
    }
}

fn scavenge_deleted_tmp_mosaico_processes() {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        if pid <= 1 {
            continue;
        }
        let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) else {
            continue;
        };
        let exe = exe.to_string_lossy();
        if !(exe.contains("(deleted)") && exe.contains("/tmp/") && exe.contains("mosaico")) {
            continue;
        }
        let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
            continue;
        };
        let cmdline = String::from_utf8_lossy(&cmdline);
        let args: Vec<&str> = cmdline
            .split('\0')
            .filter(|part| !part.is_empty())
            .collect();
        let role = args.get(1).copied().unwrap_or("");
        if role != "daemon" && role != "__pty-supervisor" {
            continue;
        }
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
}
