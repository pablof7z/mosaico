//! Isolated backend filesystem and exact binary invocation.

mod fixtures;

use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use nostr::Keys;

#[derive(Debug)]
pub struct RunResult {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub elapsed: Duration,
    pub timed_out: bool,
}

impl RunResult {
    pub fn success(&self) -> bool {
        self.status == Some(0) && !self.timed_out
    }

    pub fn combined(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

pub struct Backend {
    root: PathBuf,
    home: PathBuf,
    mosaico_home: PathBuf,
    config: PathBuf,
    work_dir: PathBuf,
    backend_secret: String,
    operator_secret: String,
    keepalives: Vec<Child>,
}

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backend")
            .field("root", &self.root)
            .field("mosaico_home", &self.mosaico_home)
            .finish()
    }
}

impl Backend {
    pub fn create(sandbox: &Path, name: &str, relay: Option<&str>, ordinal: usize) -> Result<Self> {
        let root = sandbox.join("backends").join(name);
        let home = root.join("home");
        let mosaico_home = root.join("mosaico");
        let config = root.join("config.json");
        let work_dir = root.join("work").join("workspace");
        for dir in [&home, &mosaico_home, &work_dir] {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }

        let backend_secret = format!("{:064x}", ordinal);
        let operator_secret = format!("{:064x}", ordinal + 100);
        let backend = Self {
            root,
            home,
            mosaico_home,
            config,
            work_dir,
            backend_secret,
            operator_secret,
            keepalives: Vec::new(),
        };
        backend.install_shims()?;
        if let Some(relay) = relay {
            backend.write_config(name, relay, ordinal)?;
        }
        Ok(backend)
    }

    pub fn run(&self, args: &[&str], stdin: Option<&str>, deadline: Duration) -> Result<RunResult> {
        self.run_in(args, stdin, deadline, &self.work_dir, &[])
    }

    pub fn run_in(
        &self,
        args: &[&str],
        stdin: Option<&str>,
        deadline: Duration,
        cwd: &Path,
        env: &[(&str, &str)],
    ) -> Result<RunResult> {
        let stdout_path = self.root.join("last.stdout");
        let stderr_path = self.root.join("last.stderr");
        let stdout = File::create(&stdout_path)?;
        let stderr = File::create(&stderr_path)?;
        let mut command = self.command(args);
        command
            .current_dir(cwd)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        for (name, value) in env {
            command.env(name, value);
        }
        if stdin.is_some() {
            command.stdin(Stdio::piped());
        }
        let started = Instant::now();
        let mut child = command.spawn().context("spawn exact Mosaico binary")?;
        if let Some(input) = stdin {
            child
                .stdin
                .take()
                .context("Mosaico stdin pipe")?
                .write_all(input.as_bytes())?;
        }

        let mut timed_out = false;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status.code();
            }
            if started.elapsed() >= deadline {
                timed_out = true;
                let _ = child.kill();
                break child.wait()?.code();
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        Ok(RunResult {
            status,
            stdout: std::fs::read_to_string(stdout_path).unwrap_or_default(),
            stderr: std::fs::read_to_string(stderr_path).unwrap_or_default(),
            elapsed: started.elapsed(),
            timed_out,
        })
    }

    pub fn socket(&self) -> PathBuf {
        self.mosaico_home.join("daemon.sock")
    }

    pub fn mosaico_home(&self) -> &Path {
        &self.mosaico_home
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn workspace(&self, slug: &str) -> Result<PathBuf> {
        let path = self.root.join("work").join(slug);
        std::fs::create_dir_all(&path)?;
        Ok(path)
    }

    pub fn trust_operator(&self, secret: &str) -> Result<()> {
        let mut config: serde_json::Value = serde_json::from_slice(&std::fs::read(&self.config)?)?;
        let pubkey = Keys::parse(secret)?.public_key().to_hex();
        config["userNsec"] = secret.into();
        config["whitelistedPubkeys"] = serde_json::json!([pubkey]);
        std::fs::write(&self.config, serde_json::to_vec_pretty(&config)?)?;
        Ok(())
    }

    pub fn backend_secret(&self) -> &str {
        &self.backend_secret
    }

    pub fn operator_secret(&self) -> &str {
        &self.operator_secret
    }

    pub fn backend_pubkey(&self) -> Result<String> {
        Ok(Keys::parse(&self.backend_secret)?.public_key().to_hex())
    }

    pub fn keep_channel_reader(&mut self, workspace: &str) -> Result<()> {
        let channel = format!("/{workspace}");
        let child = self
            .command(&["channel", "read", "--live", "--channel", &channel])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("start public live channel reader")?;
        self.keepalives.push(child);
        Ok(())
    }

    pub fn stop(&mut self) {
        for child in &mut self.keepalives {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.keepalives.clear();
        self.stop_daemon();
    }

    fn stop_daemon(&self) {
        if !self.socket().exists() {
            return;
        }
        let _ = self.run(&["daemon", "stop"], None, Duration::from_secs(10));
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.socket().exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mosaico"));
        command
            .env_clear()
            .args(args)
            .current_dir(&self.work_dir)
            .env("HOME", &self.home)
            .env("MOSAICO_HOME", &self.mosaico_home)
            .env("MOSAICO_CONFIG", &self.config)
            .env("MOSAICO_BIN", env!("CARGO_BIN_EXE_mosaico"))
            .env("MOSAICO_DAEMON_GRACE_S", "10")
            .env("MOSAICO_ISOLATED_HOME_OK", "1");
        for name in [
            "MOSAICO_AGENT",
            "MOSAICO_CHANNEL",
            "MOSAICO_EPHEMERAL",
            "MOSAICO_PTY_SESSION",
            "MOSAICO_PTY_SOCKET",
            "MOSAICO_PUBKEY",
            "CLAUDE_CODE_SESSION_ID",
        ] {
            command.env_remove(name);
        }
        let paths = [
            self.home.join("bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin"),
        ];
        command.env("PATH", std::env::join_paths(paths).expect("join shim PATH"));
        command
    }

    fn write_config(&self, name: &str, relay: &str, _ordinal: usize) -> Result<()> {
        let operator_pubkey = Keys::parse(&self.operator_secret)?.public_key().to_hex();
        let config = serde_json::json!({
            "whitelistedPubkeys": [operator_pubkey],
            "relays": [relay],
            "indexerRelay": relay,
            "backendName": name,
            "userNsec": self.operator_secret,
            "mosaicoPrivateKey": self.backend_secret,
            "perSessionRooms": false,
        });
        std::fs::write(&self.config, serde_json::to_vec_pretty(&config)?)?;
        Ok(())
    }
}
