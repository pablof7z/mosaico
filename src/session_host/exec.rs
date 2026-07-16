use crate::daemon::server::DaemonState;
use crate::session::Harness;
use crate::session_host::launch::workspace_abs_path;
use crate::session_host::registry::{build_headless_command, headless_shape_for_harness};
use anyhow::{Context, Result};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[path = "exec/native_id.rs"]
mod native_id;
pub(crate) use native_id::bind_native_id_from_log;
#[cfg(test)]
use native_id::extract_native_session_id;
use native_id::{fresh_native_session_id, harness_for_shape};

pub(crate) struct ExecLaunch {
    pub(crate) id: String,
    pub(crate) child: Child,
    pub(crate) log_path: PathBuf,
    pub(crate) started_at: u64,
    pub(crate) harness: String,
    pub(crate) pubkey: String,
    pub(crate) runtime_generation: u64,
}

impl ExecLaunch {
    pub(crate) fn pid(&self) -> i32 {
        self.child.id() as i32
    }
}

pub(crate) fn agent_supports_headless_exec(slug: &str) -> bool {
    let Ok(agent) = crate::identity::agent_launch_config(&crate::config::mosaico_home(), slug)
    else {
        return false;
    };
    let Ok(cfg) = crate::harness::HarnessesConfig::load() else {
        return false;
    };
    crate::harness::bundle_transport_with(&cfg, &agent.harness)
        .is_ok_and(|transport| transport == crate::harness::Transport::HeadlessExec)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn spawn_agent_exec(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    prompt: &str,
    resume_id: Option<&str>,
    group: Option<&str>,
    client_cwd: Option<&Path>,
) -> Result<ExecLaunch> {
    let agent = crate::identity::agent_launch_config(&crate::config::mosaico_home(), slug)?;
    let scratch = crate::config::mosaico_home()
        .join("harness-profiles")
        .join(slug);
    let resolved = crate::harness::resolve(&agent.harness, agent.profile.as_deref(), &scratch)?;
    if resolved.transport != crate::harness::Transport::HeadlessExec {
        anyhow::bail!(
            "agent {slug:?} uses bundle {:?} with transport {}, not headless-exec",
            agent.harness,
            resolved.transport.as_str()
        );
    }
    resolved.profile.materialize()?;
    let shape = headless_shape_for_harness(resolved.harness).with_context(|| {
        format!(
            "harness {} does not support headless exec",
            resolved.harness.as_str()
        )
    })?;
    let fresh_session_id = fresh_native_session_id(shape, resume_id)?;
    let native_id = resume_id.or(fresh_session_id.as_deref());
    let command = build_headless_command(
        &resolved.base_argv,
        shape,
        resume_id,
        fresh_session_id.as_deref(),
        prompt,
    );
    let abs_path = workspace_abs_path(state, root, client_cwd)?;
    let harness = harness_for_shape(shape);
    let identity = crate::identity::load(&crate::config::mosaico_home(), slug)?;
    let reservation = match resume_id {
        Some(native) => crate::session_host::admission::reserve_resume(
            state,
            &identity,
            harness.as_str(),
            root,
            group.unwrap_or(root),
            native,
        )?,
        None => crate::session_host::admission::reserve_fresh(
            state,
            &identity,
            harness.as_str(),
            root,
            group,
            None,
        )?,
    };
    let mut launch = match spawn_process(
        slug,
        root,
        group,
        &abs_path,
        command,
        harness,
        &reservation.pubkey,
        &reservation.agent_nsec,
        reservation.runtime_generation,
        &resolved.profile.extra_env,
    ) {
        Ok(launch) => launch,
        Err(error) => {
            crate::session_host::admission::release(state, &reservation);
            return Err(error);
        }
    };
    if let Err(e) = crate::daemon::server::session_start::bootstrap_exec_session_start(
        state,
        slug,
        harness,
        &abs_path,
        group,
        launch.pid(),
        native_id,
        &reservation.pubkey,
    )
    .await
    {
        let _ = launch.child.kill();
        crate::session_host::admission::release(state, &reservation);
        return Err(e).with_context(|| format!("registering headless exec session for {slug}"));
    }
    Ok(launch)
}

#[allow(clippy::too_many_arguments)]
fn spawn_process(
    slug: &str,
    root: &str,
    group: Option<&str>,
    cwd: &str,
    command: Vec<String>,
    harness: Harness,
    pubkey: &str,
    agent_nsec: &str,
    runtime_generation: u64,
    extra_env: &[(String, String)],
) -> Result<ExecLaunch> {
    if command.is_empty() {
        anyhow::bail!("headless exec command must not be empty");
    }
    let id = exec_id(slug);
    let started_at = crate::util::now_secs();
    let dir = exec_session_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let log_path = dir.join(format!("{id}.log"));
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("opening {}", log_path.display()))?;
    let log_err = log.try_clone()?;

    let mut child_cmd = std::process::Command::new(&command[0]);
    let mut env = extra_env.to_vec();
    let mut env_remove = Vec::new();
    crate::session_host::agent_env::assign(&mut env, &mut env_remove, pubkey, agent_nsec);
    child_cmd
        .args(&command[1..])
        .current_dir(cwd)
        .env("MOSAICO_SPAWNED", "1")
        .env("MOSAICO_AGENT", slug)
        .env_remove("MOSAICO_EPHEMERAL")
        .env_remove("MOSAICO_PTY_SESSION")
        .env_remove("MOSAICO_PTY_SOCKET")
        .env_remove("CLAUDE_CODE_CHILD_SESSION")
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    if let Some(channel) = group.filter(|g| !g.is_empty()) {
        child_cmd.env("MOSAICO_CHANNEL", channel);
    }
    child_cmd.envs(env);
    unsafe {
        child_cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = child_cmd
        .spawn()
        .with_context(|| format!("spawning headless exec for {slug} in {root}"))?;
    Ok(ExecLaunch {
        id,
        child,
        log_path,
        started_at,
        harness: harness.as_str().to_string(),
        pubkey: pubkey.to_string(),
        runtime_generation,
    })
}

#[cfg(test)]
#[path = "exec/tests.rs"]
mod tests;

fn exec_session_dir() -> PathBuf {
    crate::config::mosaico_home().join("exec-sessions")
}

fn exec_id(agent: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis();
    let safe_agent = agent
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();
    format!("exec-{safe_agent}-{now}-{}", std::process::id())
}
