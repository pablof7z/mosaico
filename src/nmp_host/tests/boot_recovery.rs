//! What a daemon boot costs over a large durable NMP publish queue (nmp#889).
//!
//! `daemon::server::lifecycle::run` opens the engine and registers the backend
//! identity as ONE step — `NmpHost::open` is `Engine::new` followed by
//! `ensure_identity` — and `add_account` is a command on the same engine
//! channel the boot rebuild owns until it finishes. So this wall time is the
//! daemon's own time-to-first-NMP-work over whatever the previous boot left
//! owed, which is exactly the laptop incident behind nmp#889.
//!
//! Ignored: it seeds thousands of durable writes, so it is a measurement run
//! on demand, never a gate. The gate for the bound itself lives in NMP
//! (`nmp::boot_recovery_bound`); this one is the consumer-side reading.
//!
//! ```text
//! cargo test --release --lib nmp_host::tests::boot_recovery -- --ignored --nocapture
//! ```

use std::time::Instant;

use super::*;

/// Configured, allowed, and never reachable — so every seeded write is
/// accepted, durably queued, and still owed at the next boot. Port 1 is
/// privileged and unbound, so the dial fails immediately rather than hanging.
const UNREACHABLE_RELAY: &str = "ws://127.0.0.1:1";

/// Comparable with NMP's own bound measurement on the same host.
const INTENTS: usize = 4_000;

#[test]
#[ignore = "seeds thousands of durable writes; a measurement, not a gate"]
fn daemon_boot_over_a_large_publish_queue() {
    let dir = tempfile::tempdir().expect("temp store dir");
    let store = dir.path().join("nmp.redb");
    let backend = Keys::generate();
    let agent = Keys::generate();

    let seeding = Instant::now();
    {
        let host = NmpHost::open(
            &[UNREACHABLE_RELAY.to_string()],
            None,
            Some(&store),
            &backend,
        )
        .expect("open the seeding host");
        for index in 0..INTENTS {
            host.publish_groups(
                [format!("room-{}", index % 64)],
                EventBuilder::new(Kind::from(30315u16), "")
                    .tags([Tag::parse(["d", "general"]).unwrap()]),
                &agent,
            )
            .expect("NMP takes custody of the write");
        }
        host.shutdown();
    }
    let seeding = seeding.elapsed();

    let boot = Instant::now();
    let host = NmpHost::open(
        &[UNREACHABLE_RELAY.to_string()],
        None,
        Some(&store),
        &backend,
    )
    .expect("reopen over the seeded queue");
    let boot = boot.elapsed();

    // What `auth_restore::restore` does for every identity the daemon holds:
    // one more `add_account` behind the same boot.
    let restore = Instant::now();
    host.ensure_identity(&agent).expect("restore one identity");
    let restore = restore.elapsed();

    let queue = host
        .engine
        .publish_queue()
        .expect("read the durable queue back");

    println!("intents seeded:      {INTENTS} in {seeding:?}");
    println!("queue after reopen:  {} entries", queue.len());
    println!("NmpHost::open:       {boot:?}");
    println!("ensure_identity:     {restore:?}");

    host.shutdown();
}

/// The same measurement over a store this machine already has, named by
/// `MOSAICO_BOOT_STORE` — for when the question is what THIS installation's
/// own `~/.mosaico/nmp.redb` costs rather than what a seeded one does.
///
/// The file is COPIED first and the copy is what boots. A measurement may not
/// open the live daemon's store: the engine takes an owner lock and writes.
#[test]
#[ignore = "needs MOSAICO_BOOT_STORE; a measurement, not a gate"]
fn daemon_boot_over_an_existing_store() {
    let source = std::env::var("MOSAICO_BOOT_STORE")
        .expect("set MOSAICO_BOOT_STORE to the nmp.redb this boot should read");
    let dir = tempfile::tempdir().expect("temp store dir");
    let store = dir.path().join("nmp.redb");
    let copied = std::fs::copy(&source, &store).expect("copy the store aside");

    let boot = Instant::now();
    let host = NmpHost::open(
        &[UNREACHABLE_RELAY.to_string()],
        None,
        Some(&store),
        &Keys::generate(),
    )
    .expect("boot over the copied store");
    let boot = boot.elapsed();

    let queue = host
        .engine
        .publish_queue()
        .expect("read the durable queue back");

    println!("store:               {source} ({copied} bytes)");
    println!("queue after boot:    {} entries", queue.len());
    println!("NmpHost::open:       {boot:?}");

    host.shutdown();
}
