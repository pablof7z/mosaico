//! Real standalone-daemon proof for the captured 207-profile observation shape.
//! Every process, database, socket, and relay is disposable and local.

#[path = "nmp_daemon_stress/process.rs"]
mod process;
#[path = "nmp_daemon_stress/relay.rs"]
mod relay;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use process::{sample, DaemonProcess};
use relay::{CountingRelay, RelaySnapshot};

const PROFILE_OBSERVATIONS: usize = 207;
// One retained group-record discovery, one group-content observation, and two
// daemon-lifetime management observations surround the narrow profile set.
const MANAGED_OBSERVATIONS: usize = PROFILE_OBSERVATIONS + 4;
const STEADY_WINDOW: Duration = Duration::from_secs(3);

#[test]
fn standalone_daemon_keeps_207_profile_observations_responsive_and_bounded() {
    let relay = CountingRelay::start(group_snapshots());
    let home = tempfile::tempdir().expect("disposable Mosaico home");
    let config = home.path().join("config.json");
    seed_product_shape(&config, &relay.url());

    let mut daemon = DaemonProcess::spawn(&binary(), home.path(), &config);
    daemon.wait_ready(Duration::from_secs(30));
    let product = wait_for_product_shape(&daemon, Duration::from_secs(20));
    assert_eq!(product["profile_observations"], PROFILE_OBSERVATIONS);
    assert_eq!(product["managed_observations"], MANAGED_OBSERVATIONS);
    let before_relay = wait_for_relay_quiet(&relay, Duration::from_secs(10));
    assert!(
        before_relay.requests > 0,
        "the product emitted no relay request"
    );
    assert!(
        before_relay.active_requests <= 40,
        "207 compatible profile observations exceeded the live wire bound: {before_relay:?}"
    );
    assert!(
        before_relay.requests <= before_relay.active_requests + 2,
        "group discovery churned more than its two predecessor requests: {before_relay:?}"
    );

    let pid = daemon.pid();
    let before_process = sample(pid);
    let window_started = Instant::now();
    let mut max_rss = before_process.rss_kib;
    let mut max_threads = before_process.threads;
    let mut max_fds = before_process.file_descriptors;
    let mut max_ping_ms = 0u128;
    let mut pings = 0usize;
    while window_started.elapsed() < STEADY_WINDOW {
        let ping_started = Instant::now();
        assert_eq!(daemon.call("ping")["pong"], true);
        max_ping_ms = max_ping_ms.max(ping_started.elapsed().as_millis());
        pings += 1;
        let current = sample(pid);
        max_rss = max_rss.max(current.rss_kib);
        max_threads = max_threads.max(current.threads);
        max_fds = max_fds.max(current.file_descriptors);
        std::thread::sleep(Duration::from_millis(250));
    }
    let elapsed = window_started.elapsed().as_secs_f64();
    let after_process = sample(pid);
    let cpu_seconds = (after_process.cpu_seconds - before_process.cpu_seconds).max(0.0);
    let steady_cpu_percent = cpu_seconds / elapsed * 100.0;
    let after_relay = relay.snapshot();

    assert!(
        pings >= 8,
        "steady window did not exercise RPC responsiveness"
    );
    assert!(max_ping_ms < 500, "ping stalled for {max_ping_ms}ms");
    assert!(
        steady_cpu_percent < 25.0,
        "idle daemon consumed {steady_cpu_percent:.1}% CPU over {elapsed:.2}s"
    );
    assert!(max_rss < 512 * 1024, "daemon RSS grew to {max_rss} KiB");
    assert!(
        max_threads < 512,
        "daemon thread count grew to {max_threads}"
    );
    assert!(max_fds < 512, "daemon descriptor count grew to {max_fds}");
    assert_eq!(after_relay.active_requests, before_relay.active_requests);
    assert_eq!(after_relay.connections, before_relay.connections);

    daemon.shutdown(Duration::from_secs(15));
    let final_relay = wait_for_relay_teardown(&relay, Duration::from_secs(5));
    assert!(!home.path().join("daemon.sock").exists());
    assert_eq!(final_relay.connections, 0);
    assert_eq!(final_relay.active_requests, 0);

    println!(
        "{}",
        serde_json::json!({
            "profile_observations": PROFILE_OBSERVATIONS,
            "managed_observations": MANAGED_OBSERVATIONS,
            "steady_cpu_percent": steady_cpu_percent,
            "steady_window_seconds": elapsed,
            "peak_rss_kib": max_rss,
            "peak_threads": max_threads,
            "peak_file_descriptors": max_fds,
            "max_ping_ms": max_ping_ms,
            "relay_requests": final_relay.requests,
            "relay_closes": final_relay.closes,
            "final_relay_connections": final_relay.connections,
            "final_active_requests": final_relay.active_requests,
        })
    );
}

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mosaico"))
}

fn seed_product_shape(config: &std::path::Path, relay: &str) {
    const BACKEND_SECRET: &str = "b53809614e9c41b923dd5546e438e48e9bcbee04b9c7c50bae0b085954e03422";
    let body = serde_json::json!({
        "whitelistedPubkeys": [],
        "backendName": "stress-host",
        "mosaicoPrivateKey": BACKEND_SECRET,
        "relays": [relay],
        "indexerRelay": relay,
    });
    std::fs::write(config, serde_json::to_vec(&body).unwrap()).expect("write stress config");
}

/// One complete relay-authored group delivery that makes the whole profile
/// demand visible through NMP. The backend plus 206 additional admins produce
/// the exact 207-profile shape without a test-only Mosaico roster writer.
fn group_snapshots() -> [Event; 2] {
    const BACKEND_SECRET: &str = "b53809614e9c41b923dd5546e438e48e9bcbee04b9c7c50bae0b085954e03422";
    let backend = Keys::parse(BACKEND_SECRET).expect("deterministic backend key");
    let relay = Keys::generate();
    let mut tags = vec![Tag::identifier("stress-root")];
    tags.push(Tag::public_key(backend.public_key()));
    for ordinal in 1..PROFILE_OBSERVATIONS {
        let secret = format!("{:064x}", ordinal + 1);
        let pubkey = Keys::parse(&secret)
            .expect("deterministic stress key")
            .public_key();
        tags.push(Tag::public_key(pubkey));
    }
    let metadata = EventBuilder::new(Kind::from(39_000), "")
        .tags([
            Tag::identifier("stress-root"),
            Tag::parse(["name", "stress-root"]).expect("group name tag"),
        ])
        .sign_with_keys(&relay)
        .expect("sign relay-authored group metadata");
    let admins = EventBuilder::new(Kind::from(39_001), "")
        .tags(tags)
        .sign_with_keys(&relay)
        .expect("sign relay-authored group admins");
    [metadata, admins]
}

fn wait_for_product_shape(daemon: &DaemonProcess, timeout: Duration) -> serde_json::Value {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = daemon.call("stress_nmp_snapshot");
        if snapshot["profile_observations"] == PROFILE_OBSERVATIONS {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "207 product observations never opened"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_relay_quiet(relay: &CountingRelay, timeout: Duration) -> RelaySnapshot {
    let deadline = Instant::now() + timeout;
    let mut previous = relay.snapshot();
    let mut unchanged = 0usize;
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let current = relay.snapshot();
        if current.requests > 0 && current.requests == previous.requests {
            unchanged += 1;
            if unchanged == 4 {
                return current;
            }
        } else {
            unchanged = 0;
        }
        previous = current;
        assert!(
            Instant::now() < deadline,
            "relay request stream never settled"
        );
    }
}

fn wait_for_relay_teardown(relay: &CountingRelay, timeout: Duration) -> RelaySnapshot {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = relay.snapshot();
        if snapshot.connections == 0 && snapshot.active_requests == 0 {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "relay connection survived daemon teardown"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
