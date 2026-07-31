//! Public session lifecycle and explicit-signer observations.

use std::time::Duration;

use super::{nak_bin, MosaicoWorld};

impl MosaicoWorld {
    pub fn stop_active_session(&mut self) {
        let pubkey = self.active_session_pubkey().to_string();
        self.send_management_command(&format!("kill {pubkey}"));
        let deadline = std::time::Instant::now() + Duration::from_secs(25);
        while std::time::Instant::now() < deadline {
            self.run(&["session", "list", "--all-workspaces", "--json"]);
            if session_rows(&self.last_run().stdout).is_some_and(|rows| {
                rows.iter()
                    .any(|row| row["pubkey"] == pubkey && row["running"] == false)
            }) {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        panic!(
            "management kill did not stop session {pubkey}\n{}",
            self.last_run().combined()
        );
    }

    pub fn same_session_is_live_without_sibling(&mut self, agent: &str) -> bool {
        let pubkey = self.active_session_pubkey().to_string();
        let deadline = std::time::Instant::now() + Duration::from_secs(25);
        while std::time::Instant::now() < deadline {
            self.run(&["session", "list", "--all-workspaces", "--json"]);
            if let Some(rows) = session_rows(&self.last_run().stdout) {
                let agent_rows = rows
                    .iter()
                    .filter(|row| row["agent"] == agent)
                    .collect::<Vec<_>>();
                if agent_rows.len() == 1
                    && agent_rows[0]["pubkey"] == pubkey
                    && agent_rows[0]["running"] == true
                {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        false
    }

    pub fn send_with_explicit_session_anchor(&mut self, body: &str) {
        let workspace = self.active_workspace().to_string();
        let explicit = self.active_session_pubkey().to_string();
        let ambient = self
            .ambient_session_pubkey
            .as_ref()
            .expect("an ambient session exists")
            .clone();
        for pubkey in [&explicit, &ambient] {
            let deadline = std::time::Instant::now() + Duration::from_secs(25);
            while std::time::Instant::now() < deadline {
                if let Ok(output) = std::process::Command::new(nak_bin())
                    .args(["req", "-k", "39002", "-d", &workspace, self.relay_url()])
                    .output()
                {
                    if output.status.success()
                        && String::from_utf8_lossy(&output.stdout).contains(pubkey.as_str())
                    {
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }
        let cwd = self
            .current_backend()
            .workspace(&workspace)
            .expect("resolve active workspace");
        let channel = format!("#{workspace}");
        let result = self
            .current_backend()
            .run_in(
                &[
                    "channel",
                    "send",
                    "--message",
                    body,
                    "--channel",
                    &channel,
                    "--session",
                    &explicit,
                ],
                None,
                Duration::from_secs(15),
                &cwd,
                &[("MOSAICO_PUBKEY", &ambient)],
            )
            .expect("send with explicit session anchor");
        assert!(
            result.success(),
            "explicit session send failed: {}",
            result.combined()
        );
        self.last_run = Some(result);
    }

    pub fn relay_message_was_authored_by_explicit_session(&self, body: &str) -> bool {
        let workspace = self.active_workspace();
        let explicit = self.active_session_pubkey();
        let deadline = std::time::Instant::now() + Duration::from_secs(25);
        while std::time::Instant::now() < deadline {
            if let Ok(output) = std::process::Command::new(nak_bin())
                .args(["req", "-k", "9", "-h", workspace, self.relay_url()])
                .output()
            {
                let expected_author = format!(r#""pubkey":"{explicit}""#);
                if output.status.success()
                    && String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .any(|line| line.contains(body) && line.contains(&expected_author))
                {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        false
    }
}

fn session_rows(output: &str) -> Option<Vec<serde_json::Value>> {
    let report: serde_json::Value = serde_json::from_str(output).ok()?;
    report["sessions"].as_array().cloned()
}
