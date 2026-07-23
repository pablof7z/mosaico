use super::*;
use std::os::unix::net::UnixListener;

fn serve_once(
    expected_command: &'static str,
    response: &'static str,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::thread::JoinHandle<()>,
) {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("pty.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let worker = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let mut command = String::new();
        reader.read_line(&mut command).unwrap();
        assert_eq!(command, expected_command);
        reader.get_mut().write_all(response.as_bytes()).unwrap();
    });
    (dir, socket, worker)
}

#[test]
fn snapshot_preserves_client_count_and_epoch() {
    let (_dir, socket, worker) = serve_once(
        "PRESENTATION\n",
        r#"{"attached_clients":2,"attachment_epoch":7,"changed_at":6}
"#,
    );
    assert_eq!(
        presentation_snapshot(socket.to_str().unwrap()).unwrap(),
        PresentationSnapshot {
            attached_clients: 2,
            attachment_epoch: 7,
            changed_at: 6,
        }
    );
    worker.join().unwrap();
}

#[test]
fn unavailable_is_not_a_headless_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let error = presentation_snapshot(dir.path().join("missing.sock").to_str().unwrap())
        .expect_err("missing supervisor must be unavailable");
    assert!(error.to_string().contains("unavailable"));
}

#[test]
fn conditional_kill_decodes_changed_presentation() {
    let (_dir, socket, worker) = serve_once(
        "KILL_IF_HEADLESS 7\n",
        r#"{"outcome":"presentation_changed","presentation":{"attached_clients":1,"attachment_epoch":8,"changed_at":9}}
"#,
    );
    assert_eq!(
        kill_if_headless_at(socket.to_str().unwrap(), 7).unwrap(),
        ConditionalKillOutcome::PresentationChanged {
            presentation: PresentationSnapshot {
                attached_clients: 1,
                attachment_epoch: 8,
                changed_at: 9,
            },
        }
    );
    worker.join().unwrap();
}
