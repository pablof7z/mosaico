use super::*;

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
    assert_eq!(
        due_retirements(&home),
        std::slice::from_ref(&old_pubkey),
        "the retirement must be durable while the relay is wedged"
    );

    // The relay recovers: point the daemon's configuration at a live relay.
    let cfg = home.dir.path().join("config.json");
    std::fs::write(
        &cfg,
        serde_json::json!({
            "whitelistedPubkeys": [],
            "backendName": "test-host",
            "relay": shared_relay_url(),
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
    stop_daemon(&home);
}
