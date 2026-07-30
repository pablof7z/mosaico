//! Deterministic native-harness setup and launch witnesses.

use std::time::Duration;

use super::MosaicoWorld;

impl MosaicoWorld {
    pub fn install_claude_profile_agent(&self, profile: &str, agent: &str) {
        self.current_backend()
            .install_claude_profile_agent(profile, agent)
            .unwrap_or_else(|error| panic!("install Claude profile agent: {error:#}"));
    }

    pub fn launch_agent(&mut self, agent: &str) {
        self.run(&[agent]);
    }

    pub fn harness_argv(&self) -> Vec<String> {
        self.current_backend()
            .harness_argv()
            .unwrap_or_else(|error| panic!("capture native harness argv: {error:#}"))
    }

    pub fn legacy_terminal_host_was_not_invoked(&self) -> bool {
        !self.current_backend().legacy_terminal_host_was_invoked()
    }

    pub fn configure_claude_agent_in_workspace(
        &mut self,
        profile: &str,
        agent: &str,
        workspace: &str,
    ) {
        let backend = self.current_backend();
        let cwd = backend.workspace(workspace).expect("create workspace");
        let initialized = backend
            .run_in(
                &["channel", "init", "--force"],
                None,
                Duration::from_secs(15),
                &cwd,
                &[],
            )
            .expect("initialize agent workspace");
        assert!(
            initialized.success(),
            "workspace initialization failed: {}",
            initialized.combined()
        );
        backend
            .install_claude_profile_agent(profile, agent)
            .expect("install profile agent");
        let launched = backend
            .run_in(&[agent], None, Duration::from_secs(15), &cwd, &[])
            .expect("launch profile agent");
        assert!(
            launched.success(),
            "profile agent launch failed: {}",
            launched.combined()
        );
        let pubkey = backend
            .harness_pubkey()
            .expect("capture launched session public key");
        self.last_run = Some(launched);
        self.active_workspace = Some(workspace.to_string());
        self.active_session_pubkey = Some(pubkey);
    }

    pub fn configure_stable_claude_agent(&mut self, agent: &str, workspace: &str) {
        self.start_agent_in_workspace("local", workspace);
        let backend = self.current_backend();
        backend
            .install_claude_profile_agent(agent, agent)
            .expect("install stable profile agent");
        let pubkey = backend
            .make_agent_stable(agent)
            .expect("configure stable agent identity");
        self.active_workspace = Some(workspace.to_string());
        self.active_session_pubkey = Some(pubkey);
    }

    pub fn configure_two_claude_agents(&mut self, first: &str, second: &str, workspace: &str) {
        self.configure_claude_agent_in_workspace(first, first, workspace);
        let first_pubkey = self
            .active_session_pubkey
            .as_ref()
            .expect("first session public key")
            .clone();
        let backend = self.current_backend();
        backend
            .install_claude_profile_agent(second, second)
            .expect("install second profile agent");
        let cwd = backend.workspace(workspace).expect("resolve workspace");
        let launched = backend
            .run_in(&[second], None, Duration::from_secs(15), &cwd, &[])
            .expect("launch second profile agent");
        assert!(
            launched.success(),
            "second profile agent launch failed: {}",
            launched.combined()
        );
        let second_pubkey = backend
            .harness_pubkey()
            .expect("capture second session public key");
        assert_ne!(
            first_pubkey, second_pubkey,
            "two per-session agents must not share a public identity"
        );
        self.last_run = Some(launched);
        self.ambient_session_pubkey = Some(first_pubkey);
        self.active_session_pubkey = Some(second_pubkey);
    }
}
