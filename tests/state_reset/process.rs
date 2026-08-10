use super::support::*;
use mosaico::daemon::protocol::{protocol_version, Response};
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct ExactChild {
    child: Child,
    role: &'static str,
    cleanup: Cleanup,
}

#[derive(Clone)]
enum Cleanup {
    KillExact,
    ReapSupervisor {
        root: PathBuf,
        instance: String,
        socket: PathBuf,
    },
}

impl ExactChild {
    fn id(&self) -> u32 {
        self.child.id()
    }

    fn is_running(&mut self) -> bool {
        self.child
            .try_wait()
            .expect("inspect exact child")
            .is_none()
    }

    fn wait_for_exit(&mut self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if !self.is_running() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    }
}

impl Drop for ExactChild {
    fn drop(&mut self) {
        if !self.is_running() {
            let _ = self.child.wait();
            return;
        }
        match self.cleanup.clone() {
            Cleanup::KillExact => {
                let _ = self.child.kill();
            }
            Cleanup::ReapSupervisor {
                root,
                instance,
                socket,
            } => {
                let _ = child_command(&root, &instance)
                    .args(["daemon", "stop"])
                    .env(mosaico::pty::REAP_SESSIONS_ON_STOP_ENV, "1")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                if self.is_running() {
                    if let Ok(mut stream) = UnixStream::connect(&socket) {
                        let _ = stream.write_all(b"KILL\n");
                        let _ = stream.flush();
                    }
                }
            }
        }
        assert!(
            self.wait_for_exit(),
            "owned {} process survived teardown",
            self.role
        );
        let _ = self.child.wait();
    }
}

fn child_command(root: &Path, instance: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mosaico"));
    command
        .env("HOME", root)
        .env("MOSAICO", instance)
        .env_remove("MOSAICO_HOME")
        .env_remove("MOSAICO_CONFIG")
        .env_remove(mosaico::pty::REAP_SESSIONS_ON_STOP_ENV)
        .stdin(Stdio::null());
    command
}

fn spawn_daemon(root: &Path, home: &Path) -> ExactChild {
    let log = std::fs::File::create(home.join("process-contract-daemon.log")).unwrap();
    let child = child_command(root, "relay1")
        .arg("daemon")
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .expect("spawn selected daemon");
    let mut daemon = ExactChild {
        child,
        role: "selected daemon",
        cleanup: Cleanup::KillExact,
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if daemon.is_running() && ping(&home.join("daemon.sock")).is_ok() {
            return daemon;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let log = std::fs::read_to_string(home.join("process-contract-daemon.log")).unwrap_or_default();
    panic!("selected daemon did not become ready: {log}");
}

fn ping(socket: &Path) -> anyhow::Result<()> {
    let stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    writeln!(
        writer,
        "{}",
        json!({"protocol": protocol_version(), "client_version": "state-reset-test"})
    )?;
    let mut welcome = String::new();
    reader.read_line(&mut welcome)?;
    writeln!(
        writer,
        "{}",
        json!({"id": 1, "method": "ping", "params": {}})
    )?;
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let response: Response = serde_json::from_str(&line)?;
    if response
        .ok
        .as_ref()
        .and_then(|value| value["pong"].as_bool())
        != Some(true)
    {
        anyhow::bail!("unexpected ping response: {line}");
    }
    Ok(())
}

fn spawn_supervisor(root: &Path, instance: &str, id: &str) -> (ExactChild, PathBuf) {
    let home = root.join(".mosaico-instances").join(instance);
    let socket = pty_socket_directory(&home).join(format!("{id}.sock"));
    let token = format!("{instance}-owned-token");
    let harness = vec!["/bin/sleep".to_string(), "30".to_string()];
    let child = child_command(root, instance)
        .args([
            "__pty-supervisor",
            "--id",
            id,
            "--instance-token",
            &token,
            "--socket",
        ])
        .arg(&socket)
        .args(["--cwd"])
        .arg(root)
        .args(["--agent", "reset-contract", "--"])
        .args(&harness)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn exact PTY supervisor");
    let metadata = json!({
        "id": id,
        "socket": socket,
        "supervisor_pid": child.id(),
        "instance_token": token,
        "child_pid": null,
        "agent": "reset-contract",
        "root": root,
        "cwd": root,
        "ephemeral": false,
        "command": harness,
    });
    write(
        &home.join("pty").join(format!("{id}.json")),
        serde_json::to_string_pretty(&metadata).unwrap().as_bytes(),
    );
    let mut supervisor = ExactChild {
        child,
        role: "PTY supervisor",
        cleanup: Cleanup::ReapSupervisor {
            root: root.to_path_buf(),
            instance: instance.to_string(),
            socket: socket.clone(),
        },
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if supervisor.is_running() && socket.exists() {
            return (supervisor, socket);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("{instance} supervisor did not become ready");
}

#[test]
fn full_reset_stops_only_the_selected_live_daemon_and_supervisor() {
    let fixture = tempfile::tempdir().expect("temporary root");
    let home = selected_home(fixture.path());
    let kept = seed_configuration(fixture.path(), &home, &home.join("tmp/attachments"));
    let sibling_home = fixture.path().join(".mosaico-instances/relay2");
    std::fs::create_dir_all(&sibling_home).unwrap();

    let mut daemon = spawn_daemon(fixture.path(), &home);
    let daemon_pid = daemon.id();
    let (mut selected, selected_socket) =
        spawn_supervisor(fixture.path(), "relay1", "selected-live");
    let selected_pid = selected.id();
    let (mut sibling, sibling_socket) = spawn_supervisor(fixture.path(), "relay2", "sibling-live");
    let sibling_pid = sibling.id();
    write(&home.join("sessions/live/runtime.json"), b"selected state");

    let reset = command(fixture.path(), &["daemon", "reset-state", CONFIRM]);
    assert!(
        reset.status.success(),
        "live reset failed: {}",
        String::from_utf8_lossy(&reset.stderr)
    );
    assert!(
        daemon.wait_for_exit(),
        "selected daemon {daemon_pid} survived"
    );
    assert!(
        selected.wait_for_exit(),
        "selected supervisor {selected_pid} survived"
    );
    assert!(
        sibling.is_running(),
        "sibling supervisor {sibling_pid} was broadly killed"
    );
    assert!(sibling_socket.exists(), "sibling PTY socket was removed");
    assert!(!selected_socket.exists(), "selected PTY socket survived");
    assert!(!home.join("state.db").exists(), "selected SQLite survived");
    assert!(
        !home.join("nmp.redb").exists(),
        "selected NMP store survived"
    );
    assert!(
        !home.join("sessions").exists(),
        "selected sessions survived"
    );
    for (path, bytes) in kept {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}
