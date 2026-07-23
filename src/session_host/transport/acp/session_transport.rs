use super::registry::remove_after_exit_confirmation;
use super::{register_child, registry, RpcTransport};
use crate::rpc_harness::{AcpClient, AppServerClient, Dialect};
use crate::session_host::transport::acp_runtime::SteerState;
use crate::session_host::transport::acp_spawn::{
    spawn_acp_prompt, spawn_app_server_steer, spawn_app_server_steer_pending, spawn_app_server_turn,
};
use crate::session_host::transport::{
    DeliveryCompletion, EndpointRef, LaunchSpec, PreparedLaunch, ResumeSpec, RpcLaunchSpec,
    SessionEndpoint, SessionTransport, TransportKind,
};
use anyhow::{Context, Result};

#[async_trait::async_trait]
impl SessionTransport for RpcTransport {
    fn kind(&self) -> TransportKind {
        self.kind
    }

    fn prepare_launch(
        &self,
        resolved: &mut crate::harness::ResolvedHarness,
        endpoint_id: String,
    ) -> Result<PreparedLaunch> {
        let configured = match resolved.transport {
            crate::harness::Transport::Acp => TransportKind::Acp,
            crate::harness::Transport::AppServer => TransportKind::AppServer,
            crate::harness::Transport::Pty => {
                anyhow::bail!("RPC transport received a PTY launch plan")
            }
        };
        if configured != self.kind {
            anyhow::bail!(
                "{} transport received a {} launch plan",
                self.kind.as_str(),
                configured.as_str()
            );
        }
        resolved.profile.materialize()?;
        let mut extra_env = resolved.profile.extra_env.clone();
        crate::host_env::apply_executable_path(&mut extra_env);
        extra_env.push((
            "MOSAICO_OBSERVED_HARNESS".to_string(),
            resolved.harness.as_str().to_string(),
        ));
        if resolved.harness == crate::session::Harness::Goose {
            crate::goose_integration::prepare_launch_env(&mut extra_env, &endpoint_id)?;
        }
        Ok(PreparedLaunch {
            pty: Default::default(),
            rpc: Some(RpcLaunchSpec {
                driver: resolved.driver,
                argv: resolved.base_argv.clone(),
                extra_env,
                harness: resolved.harness,
            }),
        })
    }

    async fn launch(&self, spec: &LaunchSpec) -> Result<SessionEndpoint> {
        let open = self.open(spec).await?;
        Ok(SessionEndpoint {
            kind: self.kind,
            endpoint_id: open.endpoint_id.clone(),
            watch_pid: open.pid.and_then(|p| i32::try_from(p).ok()),
            native_id: Some(open.native_id.clone()),
            meta: Self::synth_meta(spec, &open.endpoint_id, open.pid, &open.argv),
        })
    }

    async fn resume(&self, spec: &LaunchSpec, resume: &ResumeSpec) -> Result<SessionEndpoint> {
        if resume.native_id.is_empty() {
            anyhow::bail!("session has no resume token (not resumable)");
        }
        let (handle, dialect, updates, argv, _harness) = self.spawn_child(spec).await?;
        let cwd = std::path::PathBuf::from(&spec.abs_path);
        let handshake: Result<()> = async {
            match dialect {
                Dialect::Acp => {
                    let client = AcpClient::new(handle.clone());
                    client
                        .initialize()
                        .await
                        .map_err(|e| anyhow::anyhow!("ACP initialize (resume): {e}"))?;
                    client
                        .session_load(&resume.native_id, &cwd)
                        .await
                        .map_err(|e| anyhow::anyhow!("ACP session/load: {e}"))?;
                }
                Dialect::AppServer => {
                    let client = AppServerClient::new(handle.clone());
                    client
                        .initialize("mosaico", env!("CARGO_PKG_VERSION"))
                        .await
                        .map_err(|e| anyhow::anyhow!("app-server initialize (resume): {e}"))?;
                    client
                        .thread_resume(&resume.native_id, &cwd)
                        .await
                        .map_err(|e| anyhow::anyhow!("app-server thread/resume: {e}"))?;
                }
            }
            Ok(())
        }
        .await;
        if let Err(error) = handshake {
            return Err(Self::failed_spawn(&handle, error).await);
        }
        let endpoint_id = self.endpoint_id(&spec.slug);
        let pid = handle.pid;
        register_child(&endpoint_id, handle, resume.native_id.clone(), cwd, updates);
        Ok(SessionEndpoint {
            kind: self.kind,
            endpoint_id: endpoint_id.clone(),
            watch_pid: pid.and_then(|p| i32::try_from(p).ok()),
            native_id: Some(resume.native_id.clone()),
            meta: Self::synth_meta(spec, &endpoint_id, pid, &argv),
        })
    }

    async fn deliver(
        &self,
        ep: &EndpointRef,
        text: &str,
        submit: bool,
    ) -> Result<DeliveryCompletion> {
        if ep.kind != self.kind {
            anyhow::bail!(
                "{} transport cannot deliver to {} endpoint",
                self.kind.as_str(),
                ep.kind.as_str()
            );
        }
        let (handle, native_id, dialect, runtime) = {
            let registry = registry().lock().unwrap();
            let child = registry
                .get(&ep.endpoint_id)
                .with_context(|| format!("no RPC session registered for {:?}", ep.endpoint_id))?;
            (
                child.handle.clone(),
                child.native_id.clone(),
                child.handle.dialect,
                child.runtime.clone(),
            )
        };
        if dialect != self.dialect() {
            anyhow::bail!(
                "{} endpoint {:?} is registered as {:?}",
                self.kind.as_str(),
                ep.endpoint_id,
                dialect
            );
        }
        let text = text.to_string();
        let completion = match dialect {
            Dialect::Acp => {
                if let Ok(mut runtime) = runtime.lock() {
                    runtime.mark_turn_started();
                }
                spawn_acp_prompt(handle, native_id, text, runtime)
            }
            Dialect::AppServer => {
                // App-server has an explicit steer RPC. A submitted mention that
                // arrives during the opening turn must steer that turn rather
                // than start a concurrent one and replace its completion waiter.
                // `submit` controls PTY Enter behavior; it never overrides the
                // app-server's authoritative turn state.
                let _ = submit;
                let steer = runtime
                    .lock()
                    .ok()
                    .map(|runtime| runtime.steer_state())
                    .unwrap_or(SteerState::Idle);
                match steer {
                    SteerState::Ready(turn_id) => {
                        spawn_app_server_steer(handle, native_id, turn_id, text)
                    }
                    SteerState::AwaitingId => {
                        spawn_app_server_steer_pending(handle, native_id, text, runtime)
                    }
                    SteerState::Idle => {
                        if let Ok(mut runtime) = runtime.lock() {
                            runtime.mark_turn_started();
                        }
                        spawn_app_server_turn(handle, native_id, text, runtime)
                    }
                }
            }
        };
        Ok(completion)
    }

    fn is_live(&self, ep: &EndpointRef) -> bool {
        if ep.kind != self.kind {
            return false;
        }
        registry()
            .lock()
            .unwrap()
            .get(&ep.endpoint_id)
            .map(|child| child.handle.dialect == self.dialect() && child.handle.is_alive())
            .unwrap_or(false)
    }

    async fn kill(&self, ep: &EndpointRef) -> Result<()> {
        if ep.kind != self.kind {
            anyhow::bail!(
                "{} transport cannot kill {} endpoint",
                self.kind.as_str(),
                ep.kind.as_str()
            );
        }
        let child = {
            let registry = registry().lock().unwrap();
            let Some(child) = registry.get(&ep.endpoint_id) else {
                return Ok(());
            };
            if child.handle.dialect != self.dialect() {
                anyhow::bail!(
                    "{} endpoint {:?} is registered as {:?}",
                    self.kind.as_str(),
                    ep.endpoint_id,
                    child.handle.dialect
                );
            }
            (child.handle.clone(), child.native_id.clone())
        };
        if child.0.dialect == Dialect::Acp {
            AcpClient::new(child.0.clone())
                .session_cancel(&child.1)
                .await;
        }
        let confirmation = child.0.kill().await;
        remove_after_exit_confirmation(&ep.endpoint_id, confirmation)
            .with_context(|| format!("killing RPC endpoint {:?}", ep.endpoint_id))?;
        Ok(())
    }
}
