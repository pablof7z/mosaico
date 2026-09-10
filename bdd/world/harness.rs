//! Deterministic native-harness setup and launch witnesses.

use std::time::Duration;

use super::MosaicoWorld;

impl MosaicoWorld {
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
}
