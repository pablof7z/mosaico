use crate::daemon_harness::*;
use mosaico::daemon::client::Client;
use mosaico::state::Store;
use std::time::Duration;

#[test]
fn cli_my_session_status_sets_the_exact_pty_session_title() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();
    let pubkey = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        let response = client
            .call(
                "session_start",
                serde_json::json!({
                    "agent": "codex",
                    "harness_session": "native-title-session",
                    "cwd": "/tmp",
                    "observed_harness": "codex",
                    "admitted_bundle": "codex-pty",
                    "admitted_transport": "pty",
                    "endpoint_provenance": "launch",
                    "pty_session": "pty-title-session",
                    "endpoint_kind": "pty",
                }),
            )
            .await
            .expect("session start");
        response["pubkey"]
            .as_str()
            .expect("public session key")
            .to_string()
    });

    let title = "Researching MCP improvements around resource allocation";
    let out = run_cli_with_env(
        &home,
        &["my", "session", "status", title],
        &[
            ("MOSAICO_PTY_SESSION", "pty-title-session"),
            ("MOSAICO_OBSERVED_HARNESS", "codex"),
        ],
    );
    assert!(
        out.status.success(),
        "my session status failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Session status set"));

    let rec = Store::open(&home.store_path())
        .unwrap()
        .get_session(&pubkey)
        .unwrap()
        .expect("session row");
    assert_eq!(rec.title, title);

    assert!(
        wait_until(Duration::from_secs(20), || {
            Store::open(&home.store_path())
                .map(|s| {
                    s.live_status_for_channel(&rec.channel_h, 0)
                        .map(|rows| {
                            rows.iter()
                                .any(|row| row.pubkey == pubkey && row.title == title)
                        })
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        }),
        "my session status should publish the title as kind:30315"
    );

    stop_daemon(&home);
}
