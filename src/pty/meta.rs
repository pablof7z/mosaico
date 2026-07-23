use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchMetadata {
    pub id: String,
    pub socket: String,
    pub supervisor_pid: u32,
    pub instance_token: String,
    /// The PTY child is a separate session leader. Persisting its pid lets a
    /// daemon prove that fallback teardown stopped the harness, not just its
    /// wrapper.
    #[serde(default)]
    pub child_pid: Option<u32>,
    pub agent: String,
    pub root: String,
    pub cwd: String,
    #[serde(default)]
    pub ephemeral: bool,
    pub command: Vec<String>,
}

pub fn session_dir() -> PathBuf {
    crate::config::mosaico_home().join("pty")
}

pub fn session_socket(id: &str) -> PathBuf {
    socket_dir_for(&crate::config::mosaico_home(), current_uid()).join(format!("{id}.sock"))
}

fn socket_dir_for(mosaico_home: &std::path::Path, uid: u32) -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from("/tmp")
            .join(format!("mosaico-pty-{uid}"))
            .join(mosaico_home_hash(mosaico_home))
    }
    #[cfg(not(unix))]
    {
        std::env::temp_dir()
            .join(format!("mosaico-pty-{uid}"))
            .join(mosaico_home_hash(mosaico_home))
    }
}

#[cfg(unix)]
fn current_uid() -> u32 {
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

fn mosaico_home_hash(mosaico_home: &std::path::Path) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in mosaico_home.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn metadata_path(id: &str) -> PathBuf {
    session_dir().join(format!("{id}.json"))
}

pub fn write_metadata(meta: &LaunchMetadata) -> Result<()> {
    std::fs::create_dir_all(session_dir()).context("creating pty session directory")?;
    let bytes = serde_json::to_vec_pretty(meta)?;
    std::fs::write(metadata_path(&meta.id), bytes).context("writing pty metadata")
}

pub(super) fn record_child_pid(
    id: &str,
    instance_token: &str,
    child_pid: Option<u32>,
) -> Result<()> {
    let Some(child_pid) = child_pid else {
        anyhow::bail!("PTY child has no process id");
    };
    for _ in 0..20 {
        let path = metadata_path(id);
        if let Ok(bytes) = std::fs::read(&path) {
            let mut metadata: LaunchMetadata = serde_json::from_slice(&bytes)?;
            if metadata.instance_token != instance_token || instance_token.is_empty() {
                anyhow::bail!("PTY metadata instance token changed before child binding");
            }
            metadata.child_pid = Some(child_pid);
            return write_metadata(&metadata);
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    anyhow::bail!("PTY launch metadata did not appear before child binding")
}

pub fn remove_metadata(id: &str) -> Result<()> {
    match std::fs::remove_file(metadata_path(id)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).context("removing pty metadata"),
    }
}

pub fn read_all_metadata() -> Vec<LaunchMetadata> {
    let Ok(entries) = std::fs::read_dir(session_dir()) else {
        return Vec::new();
    };
    let mut out = entries
        .flatten()
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|bytes| serde_json::from_slice::<LaunchMetadata>(&bytes).ok())
        .collect::<Vec<_>>();
    out.sort_by(|a, b| b.id.cmp(&a.id));
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OwnedSupervisorState {
    Missing,
    Gone,
    Running,
}

/// Observe exact supervisor ownership without changing process or metadata
/// state. A live PID is reported as running only after its command line proves
/// it owns this endpoint and instance token.
pub(crate) fn owned_supervisor_state(endpoint_id: &str) -> Result<OwnedSupervisorState> {
    let Some(metadata) = read_all_metadata()
        .into_iter()
        .find(|metadata| metadata.id == endpoint_id)
    else {
        return Ok(OwnedSupervisorState::Missing);
    };
    let pid = i32::try_from(metadata.supervisor_pid).context("supervisor pid overflow")?;
    if pid <= 1 || !process_running(pid) {
        return Ok(OwnedSupervisorState::Gone);
    }
    verify_owned_process(pid, endpoint_id, &metadata)?;
    Ok(OwnedSupervisorState::Running)
}

/// Terminate only the supervisor whose persisted metadata and live command line
/// both identify `endpoint_id`. The identity check prevents a recycled PID from
/// turning stale metadata into an unrelated process kill.
pub(super) fn terminate_owned_supervisor(endpoint_id: &str) -> Result<bool> {
    let Some(metadata) = read_all_metadata()
        .into_iter()
        .find(|metadata| metadata.id == endpoint_id)
    else {
        return Ok(false);
    };
    let pid = i32::try_from(metadata.supervisor_pid).context("supervisor pid overflow")?;
    if pid <= 1 || !process_running(pid) {
        remove_metadata(endpoint_id)?;
        return Ok(true);
    }
    verify_owned_process(pid, endpoint_id, &metadata)?;
    let child_pid = metadata
        .child_pid
        .and_then(|pid| i32::try_from(pid).ok())
        .or_else(|| discover_owned_child(pid));
    let Some(child_pid) = child_pid else {
        anyhow::bail!("refusing to terminate PTY {endpoint_id:?}: owned child is unknown");
    };
    verify_owned_child(pid, child_pid)?;
    terminate_owned_child(child_pid)?;
    if process_running(pid) {
        signal(pid, nix::sys::signal::Signal::SIGTERM)?;
        wait_for_exit(pid, 20);
    }
    if process_running(pid) {
        verify_owned_process(pid, endpoint_id, &metadata)?;
        signal(pid, nix::sys::signal::Signal::SIGKILL)?;
        wait_for_exit(pid, 20);
    }
    if process_running(pid) {
        anyhow::bail!("PTY supervisor {endpoint_id:?} pid {pid} did not terminate");
    }
    remove_metadata(endpoint_id)?;
    Ok(true)
}

/// Roll back a supervisor that this process just spawned, even if its metadata
/// could not be persisted. The instance token and exact command line prove
/// ownership; stopping the supervisor before child discovery closes the race
/// where the provider could otherwise appear between inspection and teardown.
pub(crate) fn rollback_spawned_supervisor(metadata: &LaunchMetadata) -> Result<()> {
    let pid = i32::try_from(metadata.supervisor_pid).context("supervisor pid overflow")?;
    if pid <= 1 || !process_running(pid) {
        return Ok(());
    }
    verify_owned_process(pid, &metadata.id, metadata)?;
    signal(pid, nix::sys::signal::Signal::SIGSTOP)?;
    wait_for_stop(pid, 20);
    if process_running(pid) && !process_stopped(pid) {
        anyhow::bail!(
            "PTY startup supervisor {:?} pid {pid} did not stop for rollback",
            metadata.id
        );
    }
    if let Some(child_pid) = discover_owned_child(pid) {
        verify_owned_child(pid, child_pid)?;
        terminate_owned_child(child_pid)?;
    }
    if process_running(pid) {
        verify_owned_process(pid, &metadata.id, metadata)?;
        signal(pid, nix::sys::signal::Signal::SIGKILL)?;
        wait_for_exit(pid, 40);
    }
    if process_running(pid) {
        anyhow::bail!(
            "PTY startup supervisor {:?} pid {pid} did not terminate",
            metadata.id
        );
    }
    Ok(())
}

fn signal(pid: i32, signal: nix::sys::signal::Signal) -> Result<()> {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), Some(signal))
        .with_context(|| format!("sending {signal:?} to PTY supervisor pid {pid}"))
}

fn wait_for_exit(pid: i32, attempts: usize) {
    for _ in 0..attempts {
        if !process_running(pid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn wait_for_stop(pid: i32, attempts: usize) {
    for _ in 0..attempts {
        if !process_running(pid) || process_stopped(pid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

fn verify_owned_process(pid: i32, endpoint_id: &str, metadata: &LaunchMetadata) -> Result<()> {
    let command = process_command(pid)?.context("PTY supervisor is not running")?;
    if !command_owns_endpoint(&command, endpoint_id, &metadata.instance_token) {
        anyhow::bail!("refusing to terminate pid {pid}: command does not own PTY {endpoint_id:?}");
    }
    Ok(())
}

fn command_owns_endpoint(command: &str, endpoint_id: &str, instance_token: &str) -> bool {
    if endpoint_id.is_empty() || instance_token.is_empty() {
        return false;
    }
    let Some(argv) = shlex::split(command.trim()) else {
        return false;
    };
    if argv.get(1).map(String::as_str) != Some("__pty-supervisor") {
        return false;
    }
    let supervisor_args = &argv[2..];
    let option_end = supervisor_args
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(supervisor_args.len());
    let supervisor_options = &supervisor_args[..option_end];
    exact_option(supervisor_options, "--id") == Some(endpoint_id)
        && exact_option(supervisor_options, "--instance-token") == Some(instance_token)
}

fn exact_option<'a>(argv: &'a [String], option: &str) -> Option<&'a str> {
    argv.windows(2)
        .find(|pair| pair[0] == option)
        .map(|pair| pair[1].as_str())
}

fn process_command(pid: i32) -> Result<Option<String>> {
    let output = std::process::Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "state=", "-o", "command="])
        .output()
        .context("inspecting PTY supervisor command")?;
    if !output.status.success() {
        return Ok(None);
    }
    let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let Some((state, command)) = line.split_once(char::is_whitespace) else {
        return Ok(None);
    };
    if state.starts_with('Z') {
        return Ok(None);
    }
    Ok(Some(command.trim().to_string()))
}

fn process_running(pid: i32) -> bool {
    process_command(pid).ok().flatten().is_some()
}

fn process_stopped(pid: i32) -> bool {
    let Ok(output) = std::process::Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "state="])
        .output()
    else {
        return false;
    };
    output.status.success()
        && String::from_utf8_lossy(&output.stdout)
            .trim()
            .starts_with('T')
}

fn process_rows() -> Result<Vec<(i32, i32, String)>> {
    let output = std::process::Command::new("/bin/ps")
        .args(["-ax", "-o", "pid=", "-o", "ppid=", "-o", "state="])
        .output()
        .context("inspecting PTY process ownership")?;
    if !output.status.success() {
        anyhow::bail!("ps failed while inspecting PTY process ownership");
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((
                fields.next()?.parse().ok()?,
                fields.next()?.parse().ok()?,
                fields.next()?.to_string(),
            ))
        })
        .collect())
}

fn discover_owned_child(supervisor_pid: i32) -> Option<i32> {
    let mut children = process_rows()
        .ok()?
        .into_iter()
        .filter_map(|(pid, ppid, state)| {
            (ppid == supervisor_pid && !state.starts_with('Z')).then_some(pid)
        });
    let child = children.next()?;
    children.next().is_none().then_some(child)
}

fn verify_owned_child(supervisor_pid: i32, child_pid: i32) -> Result<()> {
    let owned = process_rows()?.into_iter().any(|(pid, ppid, state)| {
        pid == child_pid && ppid == supervisor_pid && !state.starts_with('Z')
    });
    if !owned {
        anyhow::bail!("refusing to terminate child pid {child_pid}: PTY ownership changed");
    }
    Ok(())
}

fn terminate_owned_child(pid: i32) -> Result<()> {
    signal(pid, nix::sys::signal::Signal::SIGHUP)?;
    wait_for_exit(pid, 10);
    if process_running(pid) {
        signal(pid, nix::sys::signal::Signal::SIGKILL)?;
        wait_for_exit(pid, 40);
    }
    if process_running(pid) {
        anyhow::bail!("PTY child pid {pid} did not terminate");
    }
    Ok(())
}

/// Resolve a PTY endpoint id to its supervisor socket without inventing a
/// socket for non-PTY endpoints. Absolute paths are accepted for direct callers.
pub fn endpoint_socket(endpoint_id: &str) -> Option<String> {
    endpoint_socket_in(endpoint_id, read_all_metadata())
}

fn endpoint_socket_in(
    endpoint_id: &str,
    metadata: impl IntoIterator<Item = LaunchMetadata>,
) -> Option<String> {
    if std::path::Path::new(endpoint_id).is_absolute() {
        return Some(endpoint_id.to_string());
    }
    metadata
        .into_iter()
        .find(|meta| meta.id == endpoint_id)
        .map(|meta| meta.socket)
}

pub fn resolve_socket(id_or_path: &str) -> PathBuf {
    let path = PathBuf::from(id_or_path);
    if path.components().count() > 1 || id_or_path.ends_with(".sock") {
        path
    } else if let Ok(bytes) = std::fs::read(metadata_path(id_or_path)) {
        serde_json::from_slice::<LaunchMetadata>(&bytes)
            .ok()
            .map(|meta| PathBuf::from(meta.socket))
            .unwrap_or_else(|| session_socket(id_or_path))
    } else {
        session_socket(id_or_path)
    }
}

#[cfg(test)]
#[path = "meta/tests.rs"]
mod tests;
