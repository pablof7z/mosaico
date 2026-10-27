use super::*;

/// The `name` of the newest kind:0 `pubkey` has on `relay`.
fn relay_profile_name(relay: &str, pubkey: &str) -> Option<String> {
    let output = std::process::Command::new(crate::common::nak_bin())
        .args(["req", "-k", "0", "-a", pubkey, "-l", "5", relay])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|event| event["pubkey"].as_str() == Some(pubkey))
        .max_by_key(|event| event["created_at"].as_u64().unwrap_or(0))
        .and_then(|event| {
            let content: serde_json::Value =
                serde_json::from_str(event["content"].as_str()?).ok()?;
            content["name"].as_str().map(str::to_string)
        })
}

fn retirement_attempts(home: &Home, pubkey: &str) -> i64 {
    rusqlite::Connection::open(home.store_path())
        .expect("open state.db")
        .query_row(
            "SELECT attempts FROM profile_retirements WHERE pubkey=?1",
            [pubkey],
            |row| row.get(0),
        )
        .unwrap_or(0)
}

fn due_retirements(home: &Home) -> Vec<String> {
    let conn = rusqlite::Connection::open(home.store_path()).expect("open state.db");
    let mut statement = conn
        .prepare("SELECT pubkey FROM profile_retirements ORDER BY pubkey")
        .expect("profile_retirements");
    statement
        .query_map([], |row| row.get(0))
        .expect("query profile_retirements")
        .collect::<rusqlite::Result<Vec<String>>>()
        .expect("profile_retirements rows")
}

/// A reclaimed handle's retirement profile is durable work: started against
/// a wedged relay it stays due, and it lands once the relay recovers.
#[test]
fn reclaimed_profile_retirement_lands_after_the_relay_recovers() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let wedged = WedgeRelay::start();
    let home = Home::with_wedged_relay(&wedged.url);

    let (old_pubkey, _) = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        let old = client
            .call(
                "session_start",
                hook_session_start(
                    serde_json::json!({
                        "agent": "claude-code",
                        "harness_session": "retire-old",
                        "cwd": "/tmp"
                    }),
                    "claude-code",
                ),
            )
            .await
            .expect("old session_start");
        let old = old["pubkey"].as_str().expect("old pubkey").to_string();
        let new = client
            .call(
                "session_start",
                hook_session_start(
                    serde_json::json!({
                        "agent": "codex",
                        "harness_session": "retire-new",
                        "reclaimed_pubkey": &old,
                        "cwd": "/tmp"
                    }),
                    "codex",
                ),
            )
            .await
            .expect("reclaiming session_start");
        (old, new)
    });
    let old_handle = Store::open(&home.store_path())
        .unwrap()
        .session_identity(&old_pubkey)
        .unwrap()
        .expect("old identity")
        .display_slug();
    assert_eq!(
        due_retirements(&home),
        std::slice::from_ref(&old_pubkey),
        "the retirement must be durable while the relay is wedged"
    );

    // The retry loop really attempted it against the wedged relay (and backed
    // off); recovery below must not wait out that backoff.
    assert!(
        wait_until(std::time::Duration::from_secs(25), || {
            retirement_attempts(&home, &old_pubkey) >= 1
        }),
        "no retirement attempt failed against the wedged relay"
    );

    // The relay recovers: point the daemon's configuration at a live relay.
    let live = shared_relay_url();
    let cfg = home.dir.path().join("config.json");
    std::fs::write(
        &cfg,
        serde_json::json!({
            "whitelistedPubkeys": [],
            "backendName": "test-host",
            "relay": &live,
        })
        .to_string(),
    )
    .unwrap();
    assert!(
        wait_until(std::time::Duration::from_secs(30), || due_retirements(
            &home
        )
        .is_empty()),
        "the retirement was not published after the relay recovered; due={:?}; daemon_log={}",
        due_retirements(&home),
        std::fs::read_to_string(home.dir.path().join("daemon.log")).unwrap_or_default()
    );
    // The relay's current profile for the old pubkey no longer carries the
    // reclaimed handle.
    let name = relay_profile_name(&live, &old_pubkey).expect("retirement kind:0 on the relay");
    assert_ne!(
        name, old_handle,
        "the old pubkey still advertises its handle"
    );
    // A retired profile names the pubkey itself (its npub), not any handle.
    let npub = nostr::nips::nip19::ToBech32::to_bech32(
        &nostr::key::PublicKey::from_hex(&old_pubkey).unwrap(),
    )
    .unwrap();
    assert_eq!(name, npub);
    stop_daemon(&home);
}
