use crate::daemon::server::DaemonState;
use crate::session::Harness;
use crate::session_host::launch::workspace_abs_path;
use crate::session_host::registry::{
    apply_agent_def_args, build_headless_command, headless_shape_for_bin, resolve_spawn_entry,
    HeadlessShape,
};
use crate::util::now_secs;
use anyhow::{Context, Result};
use std::io::Read as _;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) struct ExecLaunch {
    pub(crate) id: String,
    pub(crate) child: Child,
    pub(crate) log_path: PathBuf,
    pub(crate) started_at: u64,
    pub(crate) harness: String,
}

impl ExecLaunch {
    pub(crate) fn pid(&self) -> i32 {
        self.child.id() as i32
    }
}

pub(crate) fn agent_supports_headless_exec(slug: &str) -> bool {
    resolve_spawn_entry(slug)
        .ok()
        .and_then(|(base, _)| {
            base.first()
                .and_then(|bin| headless_shape_for_bin(bin.as_str()))
        })
        .is_some()
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn spawn_agent_exec(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    prompt: &str,
    resume_id: Option<&str>,
    base_override: Option<Vec<String>>,
    group: Option<&str>,
    client_cwd: Option<&Path>,
) -> Result<ExecLaunch> {
    let (base_command, agent_def) = match base_override {
        Some(cmd) => (cmd, None),
        None => resolve_spawn_entry(slug)?,
    };
    let base_command = if resume_id.is_some() {
        base_command
    } else {
        apply_agent_def_args(base_command, slug, agent_def)
    };
    let bin = base_command.first().map(String::as_str).unwrap_or("");
    let shape = headless_shape_for_bin(bin)
        .with_context(|| format!("agent {slug:?} does not support headless exec via {bin:?}"))?;
    let fresh_session_id = fresh_native_session_id(shape, resume_id)?;
    let native_id = resume_id.or(fresh_session_id.as_deref());
    let command = build_headless_command(
        &base_command,
        shape,
        resume_id,
        fresh_session_id.as_deref(),
        prompt,
    );
    let abs_path = workspace_abs_path(state, root, client_cwd)?;
    let harness = harness_for_shape(shape);
    let mut launch = spawn_process(slug, root, group, &abs_path, command, harness)?;
    if let Err(e) = crate::daemon::server::session_start::bootstrap_exec_session_start(
        state,
        slug,
        harness,
        &abs_path,
        group,
        launch.pid(),
        native_id,
    )
    .await
    {
        let _ = launch.child.kill();
        return Err(e).with_context(|| format!("registering headless exec session for {slug}"));
    }
    Ok(launch)
}

fn spawn_process(
    slug: &str,
    root: &str,
    group: Option<&str>,
    cwd: &str,
    command: Vec<String>,
    harness: Harness,
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
    child_cmd
        .args(&command[1..])
        .current_dir(cwd)
        .env("TENEX_EDGE_SPAWNED", "1")
        .env("TENEX_EDGE_AGENT", slug)
        .env_remove("TENEX_EDGE_SESSION")
        .env_remove("TENEX_EDGE_EPHEMERAL")
        .env_remove("TENEX_EDGE_PTY_SESSION")
        .env_remove("TENEX_EDGE_PTY_SOCKET")
        .env_remove("CLAUDE_CODE_CHILD_SESSION")
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    if let Some(channel) = group.filter(|g| !g.is_empty()) {
        child_cmd.env("TENEX_EDGE_CHANNEL", channel);
    }
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
    })
}

pub(crate) fn bind_native_id_from_log(
    state: &Arc<DaemonState>,
    session_ref: &str,
    harness: &str,
    log_path: &Path,
) {
    let Some(native_id) = extract_native_session_id(log_path) else {
        return;
    };
    if let Err(e) =
        state.with_store(|s| s.set_session_native_id(session_ref, harness, &native_id, now_secs()))
    {
        tracing::warn!(
            session = %session_ref,
            harness,
            native_id,
            error = %e,
            "failed to bind native session id from headless log"
        );
    }
}

fn harness_for_shape(shape: HeadlessShape) -> Harness {
    match shape {
        HeadlessShape::ClaudePrint => Harness::ClaudeCode,
        HeadlessShape::CodexExec => Harness::Codex,
        HeadlessShape::OpencodeRun => Harness::Opencode,
    }
}

fn fresh_native_session_id(
    shape: HeadlessShape,
    resume_id: Option<&str>,
) -> Result<Option<String>> {
    match (shape, resume_id) {
        (HeadlessShape::ClaudePrint, None) => Ok(Some(random_uuid_v4()?)),
        _ => Ok(None),
    }
}

fn random_uuid_v4() -> Result<String> {
    let mut bytes = [0_u8; 16];
    std::fs::File::open("/dev/urandom")
        .context("opening /dev/urandom")?
        .read_exact(&mut bytes)
        .context("reading random UUID bytes")?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    ))
}

pub(crate) fn extract_native_session_id(log_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(log_path).ok()?;
    content.lines().find_map(|line| {
        let value = serde_json::from_str::<serde_json::Value>(line).ok()?;
        native_id_from_value(&value)
    })
}

fn native_id_from_value(value: &serde_json::Value) -> Option<String> {
    if let Some(items) = value.as_array() {
        return items.iter().find_map(native_id_from_value);
    }
    const KEYS: &[&str] = &[
        "session_id",
        "sessionId",
        // opencode NDJSON (`run --format json`) tags every line with `sessionID`.
        "sessionID",
        "conversation_id",
        "conversationId",
        "thread_id",
        "threadId",
    ];
    let object = value.as_object()?;
    for key in KEYS {
        if let Some(id) = object.get(*key).and_then(|v| v.as_str()) {
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
    }
    if let Some(id) = object
        .get("session")
        .and_then(|v| v.get("id"))
        .and_then(|v| v.as_str())
        .filter(|id| !id.is_empty())
    {
        return Some(id.to_string());
    }
    object.values().find_map(native_id_from_value)
}

#[cfg(test)]
#[path = "exec/tests.rs"]
mod tests;

fn exec_session_dir() -> PathBuf {
    crate::config::edge_home().join("exec-sessions")
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
