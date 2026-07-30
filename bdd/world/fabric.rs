//! Relay-backed profiles, addressed messages, and management commands.

use std::time::Duration;

use anyhow::{Context as _, Result};

use super::{nak_bin, MosaicoWorld};

impl MosaicoWorld {
    pub fn keep_workspace_observation_live(&mut self, workspace: &str) {
        self.current_backend_mut()
            .keep_channel_reader(workspace)
            .unwrap_or_else(|error| panic!("keep workspace observation live: {error:#}"));
    }

    pub fn add_relay_only_peer(&mut self, name: &str, workspace: &str) {
        let peer_secret = format!("{:064x}", 500);
        let peer_pubkey = nostr::Keys::parse(&peer_secret)
            .expect("deterministic peer key")
            .public_key()
            .to_hex();
        let relay = self.relay_url().to_string();
        let profile = serde_json::json!({"name": name}).to_string();
        run_nak(&[
            "event",
            "-k",
            "0",
            "--sec",
            &peer_secret,
            "-c",
            &profile,
            &relay,
        ])
        .expect("publish relay-only peer profile");

        let backend_secret = self.current_backend().backend_secret().to_string();
        let channel_tag = format!("h={workspace}");
        let peer_tag = format!("p={peer_pubkey}");
        run_nak(&[
            "event",
            "-k",
            "9000",
            "--sec",
            &backend_secret,
            "-t",
            &channel_tag,
            "-t",
            &peer_tag,
            &relay,
        ])
        .expect("add relay-only peer to workspace");

        let deadline = std::time::Instant::now() + Duration::from_secs(25);
        while std::time::Instant::now() < deadline {
            if let Ok(output) = std::process::Command::new(nak_bin())
                .args(["req", "-k", "39002", "-d", workspace, &relay])
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if output.status.success() && stdout.contains(&peer_pubkey) {
                    self.relay_peer = Some((name.to_string(), peer_pubkey));
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        panic!("relay never confirmed peer membership in {workspace:?}");
    }

    pub fn wait_until_roster_names_peer(&mut self, expected: &str) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(25);
        while std::time::Instant::now() < deadline {
            self.run(&["who", "--all-workspaces"]);
            let needle = format!("@{expected}");
            if self.last_run().success() && self.last_run().stdout.contains(&needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        false
    }

    pub fn management_identity_is_absent_from_roster(&mut self) -> bool {
        self.run(&["who", "--all-workspaces"]);
        let public_key = self
            .current_backend()
            .backend_pubkey()
            .expect("backend public key");
        let short = &public_key[..8];
        !self.last_run().stdout.contains(&format!("@{short}"))
    }

    pub fn address_live_agent(&self, body: &str) {
        let workspace = self.active_workspace();
        let target = self.active_session_pubkey();
        assert!(
            !target.is_empty(),
            "the launched harness exposed no public key"
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(25);
        let mut admitted = false;
        while std::time::Instant::now() < deadline {
            if let Ok(output) = std::process::Command::new(nak_bin())
                .args(["req", "-k", "39002", "-d", workspace, self.relay_url()])
                .output()
            {
                if output.status.success()
                    && String::from_utf8_lossy(&output.stdout).contains(target)
                {
                    admitted = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        assert!(admitted, "target never became a relay-confirmed member");
        self.publish_addressed_message(body);
    }

    pub fn address_configured_agent(&self, body: &str) {
        self.publish_addressed_message(body);
    }

    pub fn wait_until_harness_receives_once(&self, body: &str) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(25);
        while std::time::Instant::now() < deadline {
            if self.current_backend().harness_input().matches(body).count() == 1 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        false
    }

    pub fn send_management_command(&self, body: &str) {
        let workspace = self.active_workspace();
        let backend = self.current_backend();
        let backend_pubkey = backend.backend_pubkey().expect("backend public key");
        let channel_tag = format!("h={workspace}");
        let backend_tag = format!("p={backend_pubkey}");
        run_nak(&[
            "event",
            "-k",
            "9",
            "--sec",
            backend.operator_secret(),
            "-c",
            body,
            "-t",
            &channel_tag,
            "-t",
            &backend_tag,
            self.relay_url(),
        ])
        .expect("publish backend-addressed management command");
    }

    pub fn wait_for_management_reply(&self, expected: &str) -> bool {
        let workspace = self.active_workspace();
        let deadline = std::time::Instant::now() + Duration::from_secs(25);
        while std::time::Instant::now() < deadline {
            if let Ok(output) = std::process::Command::new(nak_bin())
                .args(["req", "-k", "9", "-h", workspace, self.relay_url()])
                .output()
            {
                if output.status.success()
                    && String::from_utf8_lossy(&output.stdout).contains(expected)
                {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        false
    }

    fn publish_addressed_message(&self, body: &str) {
        let workspace = self.active_workspace();
        let target = self.active_session_pubkey();
        let backend = self.current_backend();
        let channel_tag = format!("h={workspace}");
        let target_tag = format!("p={target}");
        run_nak(&[
            "event",
            "-k",
            "9",
            "--sec",
            backend.operator_secret(),
            "-c",
            body,
            "-t",
            &channel_tag,
            "-t",
            &target_tag,
            self.relay_url(),
        ])
        .expect("publish addressed operator message");
    }
}

fn run_nak(args: &[&str]) -> Result<()> {
    let output = std::process::Command::new(nak_bin())
        .args(args)
        .output()
        .with_context(|| format!("run nak {args:?}"))?;
    anyhow::ensure!(
        output.status.success(),
        "nak {args:?} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
