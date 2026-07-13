use crate::daemon_harness::*;
use std::time::Duration;
use tenex_edge::daemon::client::Client;
use tenex_edge::state::Store;

#[test]
fn cli_my_session_status_sets_the_exact_pty_session_title() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();
    let session_id = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        let response = client
            .call(
                "session_start",
                serde_json::json!({
                    "agent": "codex",
                    "harness": "codex",
                    "session_id": "native-title-session",
                    "cwd": "/tmp",
                    "pty_session": "pty-title-session",
                }),
            )
            .await
            .expect("session start");
        response["session_id"]
            .as_str()
            .expect("canonical session id")
            .to_string()
    });

    let title = "Researching MCP improvements around resource allocation";
    let out = run_cli_with_env(
        &home,
        &["my", "session", "status", title],
        &[("TENEX_EDGE_PTY_SESSION", "pty-title-session")],
    );
    assert!(
        out.status.success(),
        "my session status failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Session status set"));
    assert!(stdout.contains("automatic distillation paused for 30 minutes"));

    let rec = Store::open(&home.store_path())
        .unwrap()
        .get_session(&session_id)
        .unwrap()
        .expect("session row");
    assert_eq!(rec.work_topic, title);
    assert!(rec.work_topic_set_at > 0);
    assert_eq!(rec.title, title);

    assert!(
        wait_until(Duration::from_secs(20), || {
            Store::open(&home.store_path())
                .map(|s| {
                    s.live_status_for_channel(&rec.channel_h, 0)
                        .map(|rows| {
                            rows.iter()
                                .any(|row| row.session_id == session_id && row.title == title)
                        })
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        }),
        "my session status should publish the title as kind:30315"
    );

    stop_daemon(&home);
}
