use super::meta::{session_socket, LaunchMetadata};
use anyhow::{Context, Result};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct SpawnSessionArgs {
    pub id: Option<String>,
    pub agent: String,
    pub project: String,
    pub cwd: PathBuf,
    pub channel: Option<String>,
    pub command: Vec<String>,
}

pub fn spawn_session(args: SpawnSessionArgs) -> Result<LaunchMetadata> {
    if args.command.is_empty() {
        anyhow::bail!("pty launch command must not be empty");
    }
    let id = args.id.unwrap_or_else(|| session_id(&args.agent));
    let socket = session_socket(&id);
    let log_path = super::meta::session_dir().join(format!("{id}.supervisor.log"));
    std::fs::create_dir_all(super::meta::session_dir())
        .context("creating pty session directory")?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("opening {}", log_path.display()))?;
    let log_err = log.try_clone()?;
    let bin = std::env::current_exe().context("locating current tenex-edge executable")?;
    let mut child = std::process::Command::new(bin);
    child
        .arg("__pty-supervisor")
        .arg("--id")
        .arg(&id)
        .arg("--socket")
        .arg(&socket)
        .arg("--cwd")
        .arg(&args.cwd)
        .arg("--agent")
        .arg(&args.agent);
    if let Some(channel) = &args.channel {
        child.arg("--channel").arg(channel);
    }
    child.arg("--").args(&args.command);
    child
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    unsafe {
        child.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let supervisor = child.spawn().context("spawning portable-pty supervisor")?;

    let meta = LaunchMetadata {
        id,
        socket: socket.to_string_lossy().to_string(),
        supervisor_pid: supervisor.id(),
        agent: args.agent,
        project: args.project,
        cwd: args.cwd.to_string_lossy().to_string(),
        command: args.command,
    };
    super::meta::write_metadata(&meta)?;
    Ok(meta)
}

fn session_id(agent: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
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
    format!("{safe_agent}-{now}-{}", std::process::id())
}
