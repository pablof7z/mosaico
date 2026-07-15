use super::*;
use crate::session_host::transport::select_transport;

#[derive(Default)]
pub(super) struct PtyLaunchSpec {
    pub(super) id: Option<String>,
    pub(super) env: Vec<(String, String)>,
    pub(super) env_remove: Vec<String>,
}

pub(super) struct ResolvedSource {
    pub(super) transport: TransportImpl,
    pub(super) command: Vec<String>,
    pub(super) harness: crate::session::Harness,
    pub(super) resume: ResumeMechanism,
    pub(super) bundle: String,
    pub(super) profile: Option<String>,
    pub(super) native_agent: Option<NativeAgentActivation>,
    pub(super) identity: crate::identity::AgentIdentity,
    pub(super) pty_launch: Option<PtyLaunchSpec>,
}

pub(super) fn resolve_agent_source(
    state: &Arc<DaemonState>,
    slug: &str,
    workspace: &std::path::Path,
) -> Result<ResolvedSource> {
    let home = crate::config::mosaico_home();
    let cfg = crate::harness::HarnessesConfig::load()?;
    let (identity, bundle, profile, native_profile) = if crate::identity::is_configured(&home, slug)
    {
        let identity = crate::identity::load(&home, slug)?;
        let bundle = identity.harness.clone();
        let profile = identity.profile.clone();
        let harness = crate::harness::bundle_harness_with(&cfg, &bundle)
            .with_context(|| format!("resolving harness for configured agent {slug:?}"))?;
        let native_profile = profile
            .is_none()
            .then(|| {
                state
                    .resolve_native_agent(slug, Some(workspace), Some(harness))
                    .ok()
            })
            .flatten();
        (identity, bundle, profile, native_profile)
    } else {
        let native_profile = state.resolve_native_agent(slug, Some(workspace), None)?;
        let bundle = crate::harness::native_bundle_with(&cfg, native_profile.harness)?;
        let identity = crate::identity::AgentIdentity::per_session(slug, &bundle);
        (identity, bundle, None, Some(native_profile))
    };
    let native_agent = native_profile
        .as_ref()
        .map(|profile| profile.activation())
        .transpose()?;
    let id = crate::pty::new_endpoint_id(slug);
    let scratch = crate::config::mosaico_home()
        .join("harness-profiles")
        .join(&id);
    let mut resolved = crate::harness::resolve_with(&cfg, &bundle, profile.as_deref(), &scratch)
        .with_context(|| format!("resolving harness bundle {bundle:?} for agent {slug:?}"))?;
    if let Some(native_agent) = &native_agent {
        crate::harness::apply_native_agent(&mut resolved, native_agent, &scratch)
            .with_context(|| format!("applying native agent {slug:?}"))?;
    }
    let transport = select_transport(&bundle)?;
    let pty_launch = if matches!(transport, TransportImpl::Pty(_)) {
        resolved.profile.materialize()?;
        let mut env = resolved.profile.extra_env.clone();
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
        Some(PtyLaunchSpec {
            id: Some(id),
            env,
            env_remove,
        })
    } else {
        None
    };
    Ok(ResolvedSource {
        transport,
        command: resolved.base_argv,
        harness: resolved.harness,
        resume: resolved.driver.resume,
        bundle,
        profile,
        native_agent,
        identity,
        pty_launch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::EnvGuard;

    fn write(path: &std::path::Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    #[tokio::test]
    async fn installed_codex_agent_resolves_without_agent_json() {
        let home = tempfile::tempdir().unwrap();
        let mosaico_home = home.path().join("mosaico");
        let codex_home = home.path().join(".codex");
        let mut env = EnvGuard::set("MOSAICO_HOME", &mosaico_home);
        env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
        env.set_var("HOME", home.path());
        env.set_var("CODEX_HOME", &codex_home);
        write(
            &mosaico_home.join("harnesses.json"),
            r#"{"codex-rpc":{"harness":"codex","transport":"app-server"}}"#,
        );
        write(
            &codex_home.join("agents/reviewer.toml"),
            "name='reviewer'\ndescription='Reviews code'\ndeveloper_instructions='Review carefully'",
        );
        let workspace = home.path().join("work");
        std::fs::create_dir_all(&workspace).unwrap();
        let state = DaemonState::new_for_test().await;
        state.refresh_agent_catalog().unwrap();

        let source = resolve_agent_source(&state, "reviewer", &workspace).unwrap();
        assert_eq!(source.bundle, "codex-rpc");
        assert!(source.identity.per_session_key);
        assert!(source.identity.keys.is_none());
        assert!(matches!(
            source.native_agent,
            Some(NativeAgentActivation::CodexRoot(_))
        ));
        assert!(!mosaico_home.join("agents/reviewer.json").exists());
    }
}
