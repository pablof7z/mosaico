use super::{pid_alive, revive_decision};

#[test]
fn nonpositive_pid_is_never_alive() {
    // Defect #3: a synth ACP pid of 0 (`kill(0)` hits the caller's own group)
    // and negative pids (`kill(-n)` hits a whole group) must read as NOT live,
    // so a dead ACP session is never treated as an immortal ghost.
    assert!(!pid_alive(0));
    assert!(!pid_alive(-1));
}

#[test]
fn dead_pid_is_never_revived() {
    assert!(!revive_decision(false, None));
    assert!(!revive_decision(false, Some(true)));
    assert!(!revive_decision(false, Some(false)));
}

#[test]
fn exec_session_revives_on_pid_alone() {
    // No PTY socket => PID liveness is authoritative.
    assert!(revive_decision(true, None));
}

#[test]
fn live_pid_with_live_pty_is_revived() {
    assert!(revive_decision(true, Some(true)));
}

#[test]
fn live_pid_with_dead_pty_is_not_revived() {
    // Guards against PID recycling: the process at `child_pid` is alive but
    // its supervisor socket is gone, so it is not our session.
    assert!(!revive_decision(true, Some(false)));
}

/// Defect #4: a LIVE session retired because its identity config changed must
/// have its PTY supervisor KILLED, not orphaned. We stand in a real listener
/// for the supervisor socket and assert the retirement path connects and
/// writes a KILL frame to it, and that the row is marked dead.
#[tokio::test]
async fn config_change_retirement_kills_supervisor_and_marks_dead() {
    use std::io::Read;
    use std::os::unix::net::UnixListener;

    let sock_path = std::env::temp_dir().join(format!(
        "te-test-kill-{}-{}.sock",
        std::process::id(),
        crate::util::now_millis()
    ));
    let _ = std::fs::remove_file(&sock_path);
    let listener = UnixListener::bind(&sock_path).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let accept = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = Vec::new();
            let _ = stream.read_to_end(&mut buf);
            let _ = tx.send(String::from_utf8_lossy(&buf).to_string());
        }
    });

    let state = crate::daemon::server::DaemonState::new_for_test().await;
    let sock_str = sock_path.to_str().unwrap().to_string();
    let sid = state
        .with_store(|s| {
            let sid = s.register_session(&crate::state::RegisterSession {
                harness: "claude-code".into(),
                external_id_kind: "harness_session".into(),
                external_id: "x1".into(),
                agent_pubkey: "pk".into(),
                agent_slug: "agent".into(),
                channel_h: "h1".into(),
                child_pid: Some(4242),
                transcript_path: None,
                resume_id: String::new(),
                now: 1000,
            })?;
            s.put_alias("claude-code", "pty_session", &sock_str, &sid, 1000)?;
            Ok::<_, anyhow::Error>(sid)
        })
        .unwrap();

    super::retire_live_session_for_config_change(&state, &sid, Some(4242), 2000);

    let received = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("retirement must connect to the supervisor socket");
    assert!(
        received.contains("KILL"),
        "retirement must send a KILL frame, got {received:?}"
    );
    accept.join().unwrap();
    let _ = std::fs::remove_file(&sock_path);

    assert!(
        !state
            .with_store(|s| s.get_session(&sid))
            .unwrap()
            .unwrap()
            .alive,
        "the retired session row must be marked dead"
    );
}
