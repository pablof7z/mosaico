//! Portable-PTY implementation of the complete hosted-session transport seam.

use anyhow::Result;

use super::{
    DeliveryCompletion, EndpointDescriptor, EndpointRef, LaunchSpec, ResumeSpec, SessionEndpoint,
    SessionTransport, TransportKind,
};

pub struct PtyTransport;

#[async_trait::async_trait]
impl SessionTransport for PtyTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::Pty
    }

    fn prepare_launch(
        &self,
        resolved: &mut crate::harness::ResolvedHarness,
        endpoint_id: String,
    ) -> Result<super::PreparedLaunch> {
        resolved.profile.materialize()?;
        let mut env = resolved.profile.extra_env.clone();
        crate::host_env::apply_executable_path(&mut env);
        let mut env_remove = Vec::new();
        for directive in resolved.driver.base_env {
            match directive {
                crate::harness::EnvDirective::Set(key, value) => {
                    env.push((key.to_string(), value.to_string()));
                }
                crate::harness::EnvDirective::Remove(key) => {
                    env_remove.push(key.to_string());
                }
            }
        }
        if resolved.harness == crate::session::Harness::Goose {
            crate::goose_integration::prepare_launch_env(&mut env, &endpoint_id)?;
        }
        env.push((
            "MOSAICO_OBSERVED_HARNESS".to_string(),
            resolved.harness.as_str().to_string(),
        ));
        Ok(super::PreparedLaunch {
            pty: super::PtyLaunchSpec {
                id: Some(endpoint_id),
                env,
                env_remove,
            },
            rpc: None,
        })
    }

    async fn launch(&self, spec: &LaunchSpec) -> Result<SessionEndpoint> {
        let mut env = spec.prepared.pty.env.clone();
        let mut env_remove = spec.prepared.pty.env_remove.clone();
        crate::session_host::agent_env::assign(
            &mut env,
            &mut env_remove,
            &spec.pubkey,
            &spec.agent_nsec,
        );
        let meta = crate::pty::spawn_session(crate::pty::SpawnSessionArgs {
            id: spec.prepared.pty.id.clone(),
            agent: spec.slug.clone(),
            root: spec.root.clone(),
            cwd: std::path::PathBuf::from(&spec.abs_path),
            channel: spec.group.clone().filter(|group| !group.is_empty()),
            session_name: spec.session_name.clone(),
            ephemeral: spec.ephemeral,
            command: spec.base_command.clone(),
            env,
            env_remove,
        })?;
        Ok(SessionEndpoint {
            kind: TransportKind::Pty,
            endpoint_id: meta.id.clone(),
            watch_pid: i32::try_from(meta.supervisor_pid).ok(),
            // PTY doesn't own a native resume token; the wrapped harness reports
            // its own session id through the mosaico hook.
            native_id: None,
            meta,
        })
    }

    async fn resume(&self, spec: &LaunchSpec, _resume: &ResumeSpec) -> Result<SessionEndpoint> {
        self.launch(spec).await
    }

    async fn deliver(
        &self,
        ep: &EndpointRef,
        text: &str,
        submit: bool,
    ) -> Result<DeliveryCompletion> {
        if !self.is_live(ep) {
            anyhow::bail!("pty session {} is not live", ep.endpoint_id);
        }
        crate::pty::inject(&ep.endpoint_id, text, true, false)?;
        if submit {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            crate::pty::inject(&ep.endpoint_id, "", false, true)?;
        }
        Ok(DeliveryCompletion::ExternallyObserved)
    }

    fn is_live(&self, ep: &EndpointRef) -> bool {
        crate::pty::is_live(&ep.endpoint_id)
    }

    fn output_is_visible(&self, ep: &EndpointRef) -> bool {
        crate::pty::presentation_observation(&ep.endpoint_id)
            .map(|presentation| !presentation.is_headless())
            .unwrap_or(false)
    }

    fn describe(&self, ep: &EndpointRef) -> EndpointDescriptor {
        let live = self.is_live(ep);
        let metadata = crate::pty::read_all_metadata()
            .into_iter()
            .find(|metadata| metadata.id == ep.endpoint_id);
        EndpointDescriptor {
            id: ep.endpoint_id.clone(),
            kind: TransportKind::Pty,
            live,
            attachable: live && metadata.is_some(),
            cwd: metadata.as_ref().map(|metadata| metadata.cwd.clone()),
            command: metadata
                .map(|metadata| metadata.command)
                .unwrap_or_default(),
        }
    }

    fn opening_delivery_delay(&self) -> std::time::Duration {
        std::time::Duration::from_millis(2000)
    }

    async fn kill(&self, ep: &EndpointRef) -> Result<()> {
        if !self.is_live(ep) {
            return Ok(());
        }
        crate::pty::kill(&ep.endpoint_id)
    }
}
