use crate::daemon_harness::*;
use mosaico::daemon::client::Client;
use mosaico::state::Store;
use std::time::Duration;

#[test]
fn cli_my_session_status_sets_the_exact_pty_session_title() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new().with_backend_key();
    configure_pty_agent(&home, "codex", "forever");
    let pty_id = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        let response = client
            .call(
                "pty_spawn",
                serde_json::json!({
                    "agent": "codex",
                    "root": "tmp",
                    "channel": "tmp",
                    "cwd": "/tmp",
                }),
            )
            .await
            .expect("pty spawn");
        response["pty_id"].as_str().expect("pty id").to_string()
    });
    let mut session = None;
    assert!(
        wait_until(Duration::from_secs(10), || {
            session = Store::open(&home.store_path())
                .and_then(|store| store.list_running_sessions())
                .unwrap_or_default()
                .into_iter()
                .find(|row| row.agent_slug == "codex");
            session.is_some()
        }),
        "spawned PTY session did not become live"
    );
    let pubkey = session.unwrap().pubkey;

    let title = "Researching MCP improvements around resource allocation";
    let out = run_cli_with_env(
        &home,
        &["my", "session", "status", title],
        &[
            ("MOSAICO_PTY_SESSION", &pty_id),
            ("MOSAICO_OBSERVED_HARNESS", "opencode"),
        ],
    );
    assert!(
        out.status.success(),
        "my session status failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Session status set"));

    let store = Store::open(&home.store_path()).unwrap();
    let rec = store.get_session(&pubkey).unwrap().expect("session row");
    assert_eq!(rec.title, title);

    let _ = mosaico::pty::kill(&pty_id);
    stop_daemon(&home);
}
