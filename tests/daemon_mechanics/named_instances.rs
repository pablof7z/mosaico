use super::*;
use mosaico::daemon::protocol::{protocol_version, Response};
use nostr::Keys;
use std::process::{Child, Stdio};

struct NamedDaemon {
    child: Child,
    home: PathBuf,
    socket: PathBuf,
}

impl NamedDaemon {
    fn start(root: &std::path::Path, instance: &str, relay: &str, backend: &str) -> Self {
        let home = root.join(".mosaico-instances").join(instance);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join("config.json"),
            serde_json::json!({
                "whitelistedPubkeys": [],
                "backendName": backend,
                "relays": [relay],
                "indexerRelay": relay,
                "mosaicoPrivateKey": Keys::generate().secret_key().to_secret_hex(),
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            home.join("workspaces.json"),
            serde_json::json!({"work": root}).to_string(),
        )
        .unwrap();
        let log = std::fs::File::create(home.join("test-daemon.log")).unwrap();
        let child = std::process::Command::new(bin())
            .arg("daemon")
            .env("HOME", root)
            .env("MOSAICO", instance)
            .env_remove("MOSAICO_HOME")
            .env_remove("MOSAICO_CONFIG")
            .env(mosaico::pty::REAP_SESSIONS_ON_STOP_ENV, "1")
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap();
        let daemon = Self {
            child,
            socket: home.join("daemon.sock"),
            home,
        };
        daemon.wait_until_ready();
        daemon
    }

    fn wait_until_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if rpc(&self.socket, "ping", serde_json::json!({})).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let log = std::fs::read_to_string(self.home.join("test-daemon.log")).unwrap_or_default();
        panic!("named daemon did not become ready: {log}");
    }
}

impl Drop for NamedDaemon {
    fn drop(&mut self) {
        let _ = rpc(&self.socket, "shutdown", serde_json::json!({}));
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn rpc(
    socket: &std::path::Path,
    method: &str,
    params: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let stream = UnixStream::connect(socket)?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    writeln!(
        writer,
        "{}",
        serde_json::json!({"protocol": protocol_version(), "client_version": "named-test"})
    )?;
    let mut welcome = String::new();
    reader.read_line(&mut welcome)?;
    writeln!(
        writer,
        "{}",
        serde_json::json!({"id": 1, "method": method, "params": params})
    )?;
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let response: Response = serde_json::from_str(&line)?;
    if let Some(error) = response.error {
        anyhow::bail!("{}: {}", error.code, error.message);
    }
    response
        .ok
        .ok_or_else(|| anyhow::anyhow!("missing RPC result"))
}

fn run_session_start_hook(root: &std::path::Path, instance: &str) -> std::process::Output {
    let mut child = std::process::Command::new(bin())
        .args(["harness", "hook", "codex", "--type", "session-start"])
        .current_dir(root)
        .env("HOME", root)
        .env("MOSAICO", instance)
        .env("MOSAICO_OBSERVED_HARNESS", "codex")
        .env("MOSAICO_INIT_PROGRESS", "0")
        .env_remove("MOSAICO_HOME")
        .env_remove("MOSAICO_CONFIG")
        .env_remove("MOSAICO_AGENT")
        .env_remove("MOSAICO_PUBKEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            serde_json::json!({
                "cwd": root,
                "session_id": "only-on-relay1",
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn two_named_daemons_have_disjoint_identity_and_sessions() {
    let root = tempfile::tempdir().unwrap();
    let relay1 = TestRelay::start();
    let relay2 = TestRelay::start();
    let daemon1 = NamedDaemon::start(root.path(), "relay1", &relay1.url, "backend-one");
    let daemon2 = NamedDaemon::start(root.path(), "relay2", &relay2.url, "backend-two");

    let backend1 = rpc(&daemon1.socket, "local_backend", serde_json::json!({})).unwrap();
    let backend2 = rpc(&daemon2.socket, "local_backend", serde_json::json!({})).unwrap();
    assert_eq!(backend1["backend_label"], "backend-one");
    assert_eq!(backend2["backend_label"], "backend-two");
    assert_ne!(backend1["pubkey"], backend2["pubkey"]);

    let hook = run_session_start_hook(root.path(), "relay1");
    assert!(
        hook.status.success(),
        "selected hook failed: {}",
        String::from_utf8_lossy(&hook.stderr)
    );
    let sessions1 = rpc(&daemon1.socket, "operator_sessions", serde_json::json!({})).unwrap();
    let sessions2 = rpc(&daemon2.socket, "operator_sessions", serde_json::json!({})).unwrap();
    assert_eq!(sessions1["sessions"].as_array().unwrap().len(), 1);
    assert!(sessions2["sessions"].as_array().unwrap().is_empty());

    let stopped = std::process::Command::new(bin())
        .args(["daemon", "stop"])
        .env("HOME", root.path())
        .env("MOSAICO", "relay1")
        .env_remove("MOSAICO_HOME")
        .env_remove("MOSAICO_CONFIG")
        .output()
        .unwrap();
    assert!(
        stopped.status.success(),
        "selected stop failed: {}",
        String::from_utf8_lossy(&stopped.stderr)
    );
    assert!(rpc(&daemon1.socket, "ping", serde_json::json!({})).is_err());
    assert_eq!(
        rpc(&daemon2.socket, "ping", serde_json::json!({})).unwrap()["pong"],
        true
    );
    let absent_hook = run_session_start_hook(root.path(), "relay1");
    assert!(
        absent_hook.status.success(),
        "absent selected daemon must fail open"
    );
    let sessions2 = rpc(&daemon2.socket, "operator_sessions", serde_json::json!({})).unwrap();
    assert!(sessions2["sessions"].as_array().unwrap().is_empty());

    assert!(!root.path().join(".mosaico").exists());
}
