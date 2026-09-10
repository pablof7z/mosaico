//! Public session lifecycle and explicit-signer observations.

use std::time::Duration;

use super::MosaicoWorld;

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
}

fn session_rows(output: &str) -> Option<Vec<serde_json::Value>> {
    let report: serde_json::Value = serde_json::from_str(output).ok()?;
    report["sessions"].as_array().cloned()
}
