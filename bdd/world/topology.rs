//! Scenario topology, isolated backends, and core process observations.

use std::time::Duration;

use super::{MosaicoWorld, RelayFixture};

impl MosaicoWorld {
    pub fn isolated_with_nak(&mut self) {
        self.ensure_sandbox();
        let root = self.root().to_path_buf();
        self.relay = Some(RelayFixture::start_nak(&root).expect("start nak relay"));
        self.add_backend("local", true)
            .expect("create configured backend");
        self.current_backend = Some("local".to_string());
    }

    pub fn isolated_with_croissant(&mut self) {
        self.ensure_sandbox();
        self.start_croissant();
        self.add_backend("local", true)
            .expect("create configured backend");
        self.current_backend = Some("local".to_string());
    }

    pub fn start_croissant(&mut self) {
        self.ensure_sandbox();
        assert!(self.relay.is_none(), "the scenario already has a relay");
        let root = self.root().to_path_buf();
        self.relay = Some(RelayFixture::start_croissant(&root).expect("start Croissant relay"));
    }

    pub fn add_isolated_backend(&mut self, name: &str) {
        self.ensure_sandbox();
        self.add_backend(name, true)
            .unwrap_or_else(|error| panic!("create backend {name:?}: {error:#}"));
        self.current_backend = Some(name.to_string());
    }

    pub fn select_backend(&mut self, name: &str) {
        assert!(self.backends.contains_key(name), "unknown backend {name:?}");
        self.current_backend = Some(name.to_string());
    }

    pub fn trust_shared_operator(&self, names: &[&str]) {
        let secret = format!("{:064x}", 900);
        for name in names {
            self.backends
                .get(*name)
                .unwrap_or_else(|| panic!("unknown backend {name:?}"))
                .trust_operator(&secret)
                .unwrap_or_else(|error| panic!("configure shared operator: {error:#}"));
        }
    }

    pub fn start_agent_in_workspace(&mut self, backend_name: &str, workspace: &str) {
        let backend = self
            .backends
            .get(backend_name)
            .unwrap_or_else(|| panic!("unknown backend {backend_name:?}"));
        let cwd = backend.workspace(workspace).expect("create workspace");
        let initialized = backend
            .run_in(
                &["channel", "init", "--force"],
                None,
                Duration::from_secs(30),
                &cwd,
                &[],
            )
            .expect("initialize workspace");
        assert!(
            initialized.success(),
            "workspace initialization failed: {}",
            initialized.combined()
        );
        let payload = format!(
            r#"{{"session_id":"bdd-{backend_name}-{workspace}","cwd":"{}"}}"#,
            cwd.display()
        );
        let started = backend
            .run_in(
                &["harness", "hook", "claude-code", "--type", "session-start"],
                Some(&payload),
                Duration::from_secs(10),
                &cwd,
                &[("MOSAICO_AGENT", "claude")],
            )
            .expect("start native session");
        assert!(
            started.success(),
            "session-start hook failed: {}",
            started.combined()
        );
        self.current_backend = Some(backend_name.to_string());
        self.last_run = Some(started);
    }

    pub fn list_channels_on(&mut self, backend_name: &str) {
        self.select_backend(backend_name);
        self.run(&["channel", "list", "--all"]);
    }

    pub fn wait_until_backend_lists(&mut self, backend_name: &str, workspace: &str) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(25);
        while std::time::Instant::now() < deadline {
            self.list_channels_on(backend_name);
            let run = self.last_run();
            if run.success() && run.stdout.contains(&format!("#{workspace}")) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        false
    }

    pub fn relay_holds_root(&self, workspace: &str) -> bool {
        let relay = self.relay_url();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            let output = std::process::Command::new(super::nak_bin())
                .args(["req", "-k", "39000", "-d", workspace, relay])
                .output();
            if let Ok(output) = output {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if output.status.success() && stdout.contains("\"kind\":39000") {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        false
    }

    pub fn backends_are_filesystem_isolated(&self, names: &[&str]) -> bool {
        let roots = names
            .iter()
            .map(|name| {
                self.backends
                    .get(*name)
                    .unwrap_or_else(|| panic!("unknown backend {name:?}"))
                    .root()
            })
            .collect::<Vec<_>>();
        roots.iter().enumerate().all(|(index, root)| {
            roots
                .iter()
                .skip(index + 1)
                .all(|other| !root.starts_with(other) && !other.starts_with(root))
        })
    }
}
