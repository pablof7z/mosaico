//! One scenario-owned Mosaico topology and its public observations.

mod artifacts;
mod backend;
mod fabric;
mod harness;
mod relay;
mod sessions;
mod topology;

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result};

use backend::Backend;
pub use backend::RunResult;
use relay::{nak_bin, RelayFixture};

#[derive(cucumber::World, Default)]
pub struct MosaicoWorld {
    sandbox: Option<tempfile::TempDir>,
    relay: Option<RelayFixture>,
    backends: BTreeMap<String, Backend>,
    current_backend: Option<String>,
    last_run: Option<RunResult>,
    relay_peer: Option<(String, String)>,
    active_workspace: Option<String>,
    active_session_pubkey: Option<String>,
    ambient_session_pubkey: Option<String>,
}

impl fmt::Debug for MosaicoWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MosaicoWorld")
            .field("sandbox", &self.sandbox.as_ref().map(|dir| dir.path()))
            .field("relay", &self.relay.as_ref().map(RelayFixture::url))
            .field("backends", &self.backends.keys().collect::<Vec<_>>())
            .field("current_backend", &self.current_backend)
            .field("last_run", &self.last_run)
            .field(
                "relay_peer",
                &self.relay_peer.as_ref().map(|(name, _)| name),
            )
            .field("active_workspace", &self.active_workspace)
            .field(
                "active_session_pubkey",
                &self
                    .active_session_pubkey
                    .as_ref()
                    .map(|pubkey| &pubkey[..8]),
            )
            .field(
                "ambient_session_pubkey",
                &self
                    .ambient_session_pubkey
                    .as_ref()
                    .map(|pubkey| &pubkey[..8]),
            )
            .finish()
    }
}

impl MosaicoWorld {
    pub fn run(&mut self, args: &[&str]) {
        let backend = self.current_backend();
        self.last_run = Some(
            backend
                .run(args, None, Duration::from_secs(15))
                .unwrap_or_else(|error| panic!("run mosaico {args:?}: {error:#}")),
        );
    }

    pub fn run_with_stdin(&mut self, args: &[&str], stdin: &str, deadline: Duration) {
        let backend = self.current_backend();
        self.last_run = Some(
            backend
                .run(args, Some(stdin), deadline)
                .unwrap_or_else(|error| panic!("run mosaico {args:?}: {error:#}")),
        );
    }

    pub fn last_run(&self) -> &RunResult {
        self.last_run.as_ref().expect("a command has run")
    }

    pub fn daemon_socket_exists(&self) -> bool {
        self.current_backend().socket().exists()
    }

    pub fn current_home(&self) -> &Path {
        self.current_backend().mosaico_home()
    }

    pub fn relay_url(&self) -> &str {
        self.relay.as_ref().expect("a relay exists").url()
    }

    fn active_workspace(&self) -> &str {
        self.active_workspace
            .as_deref()
            .expect("an active workspace exists")
    }

    fn active_session_pubkey(&self) -> &str {
        self.active_session_pubkey
            .as_deref()
            .expect("an active session public key exists")
    }

    fn ensure_sandbox(&mut self) {
        if self.sandbox.is_none() {
            self.sandbox = Some(tempfile::tempdir().expect("create BDD sandbox"));
        }
    }

    fn root(&self) -> &Path {
        self.sandbox.as_ref().expect("sandbox exists").path()
    }

    fn add_backend(&mut self, name: &str, configured: bool) -> Result<()> {
        if self.backends.contains_key(name) {
            anyhow::bail!("backend {name:?} already exists");
        }
        let relay = configured
            .then(|| {
                self.relay
                    .as_ref()
                    .context("configured backend needs a relay")
            })
            .transpose()?
            .map(RelayFixture::url);
        let ordinal = self.backends.len() + 10;
        let backend = Backend::create(self.root(), name, relay, ordinal)?;
        self.backends.insert(name.to_string(), backend);
        Ok(())
    }

    fn current_backend(&self) -> &Backend {
        let name = self
            .current_backend
            .as_ref()
            .expect("a current backend is selected");
        self.backends.get(name).expect("selected backend exists")
    }

    fn current_backend_mut(&mut self) -> &mut Backend {
        let name = self
            .current_backend
            .as_ref()
            .expect("a current backend is selected")
            .clone();
        self.backends
            .get_mut(&name)
            .expect("selected backend exists")
    }
}

impl Drop for MosaicoWorld {
    fn drop(&mut self) {
        for backend in self.backends.values_mut() {
            backend.stop();
        }
    }
}
