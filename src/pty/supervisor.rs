use anyhow::{bail, Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[path = "supervisor/clients.rs"]
mod clients;
#[path = "supervisor/session_exit.rs"]
mod session_exit;

use clients::{attach_client, fanout, output_mode, Clients};

const BACKLOG_LIMIT: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct SupervisorArgs {
    pub id: String,
    pub socket: PathBuf,
    pub cwd: PathBuf,
    pub agent: String,
    pub channel: Option<String>,
    pub session_name: Option<String>,
    pub ephemeral: bool,
    pub command: Vec<String>,
}

pub fn run_supervisor(args: SupervisorArgs) -> Result<()> {
    let _session_exit_guard = session_exit::SessionExitGuard::new(args.id.clone());
    if args.command.is_empty() {
        bail!("pty supervisor command must not be empty");
    }
    if let Some(parent) = args.socket.parent() {
        std::fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let _ = std::fs::remove_file(&args.socket);
    let listener = UnixListener::bind(&args.socket)
        .with_context(|| format!("binding {}", args.socket.display()))?;
    listener.set_nonblocking(true)?;

    let pair = native_pty_system().openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let mut cmd = CommandBuilder::from_argv(args.command.iter().map(OsString::from).collect());
    cmd.cwd(&args.cwd);
    cmd.env("MOSAICO_SPAWNED", "1");
    cmd.env("MOSAICO_AGENT", &args.agent);
    cmd.env("MOSAICO_PTY_SESSION", &args.id);
    cmd.env("MOSAICO_PTY_SOCKET", args.socket.as_os_str());
    if let Ok(pubkey) = std::env::var("MOSAICO_PUBKEY") {
        cmd.env("MOSAICO_PUBKEY", pubkey);
    } else {
        cmd.env_remove("MOSAICO_PUBKEY");
    }
    if let Ok(nsec) = std::env::var("AGENT_NSEC") {
        cmd.env("AGENT_NSEC", nsec);
    } else {
        cmd.env_remove("AGENT_NSEC");
    }
    if args.ephemeral {
        cmd.env("MOSAICO_EPHEMERAL", "1");
    } else {
        cmd.env_remove("MOSAICO_EPHEMERAL");
    }
    let term = std::env::var("TERM")
        .ok()
        .filter(|term| !term.is_empty() && term != "dumb")
        .unwrap_or_else(|| "xterm-256color".to_string());
    cmd.env("TERM", term);
    cmd.env("COLORTERM", "truecolor");
    cmd.env("CLICOLOR", "1");
    cmd.env("CLICOLOR_FORCE", "1");
    cmd.env("FORCE_COLOR", "1");
    cmd.env_remove("NO_COLOR");
    if let Some(channel) = &args.channel {
        cmd.env("MOSAICO_CHANNEL", channel);
    }
    if let Some(session_name) = &args.session_name {
        cmd.env("MOSAICO_SESSION_NAME", session_name);
    } else {
        cmd.env_remove("MOSAICO_SESSION_NAME");
    }
    cmd.env_remove("CLAUDE_CODE_SESSION_ID");
    cmd.env_remove("CLAUDE_CODE_CHILD_SESSION");

    let mut child = pair.slave.spawn_command(cmd)?;
    let killer = Arc::new(Mutex::new(child.clone_killer()));
    let writer = Arc::new(Mutex::new(pair.master.take_writer()?));
    let clients: Clients = Arc::new(Mutex::new(Vec::new()));
    let backlog = Arc::new(Mutex::new(VecDeque::with_capacity(BACKLOG_LIMIT)));

    {
        let mut reader = pair.master.try_clone_reader()?;
        let clients = clients.clone();
        let backlog = backlog.clone();
        std::thread::spawn(move || {
            let mut buf = [0_u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                remember(&backlog, &buf[..n]);
                fanout(&clients, &buf[..n]);
            }
        });
    }

    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                stream
                    .set_nonblocking(false)
                    .context("setting accepted pty client socket blocking")?;
                if let Err(e) = handle_client(
                    stream,
                    pair.master.as_ref(),
                    &writer,
                    &killer,
                    &clients,
                    &backlog,
                ) {
                    eprintln!("[mosaico pty supervisor] client error: {e:#}");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e).context("accepting pty client"),
        }
    }
    let _ = std::fs::remove_file(&args.socket);
    let _ = super::meta::remove_metadata(&args.id);
    Ok(())
}

fn handle_client(
    stream: UnixStream,
    master: &(dyn portable_pty::MasterPty + Send),
    writer: &Arc<Mutex<Box<dyn Write + Send>>>,
    killer: &Arc<Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    clients: &Clients,
    backlog: &Arc<Mutex<VecDeque<u8>>>,
) -> Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let parts = line.split_whitespace().collect::<Vec<_>>();
    match parts.as_slice() {
        ["ATTACH", rows, cols] => {
            if let (Ok(rows), Ok(cols)) = (rows.parse::<u16>(), cols.parse::<u16>()) {
                let _ = master.resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
            attach_client(reader, writer.clone(), clients, backlog)
        }
        ["INJECT", len] => {
            let len = len.parse::<usize>().context("invalid INJECT length")?;
            let mut payload = vec![0_u8; len];
            reader.read_exact(&mut payload)?;
            trace_bytes("supervisor inject", &payload);
            let mut writer = writer.lock().unwrap();
            writer.write_all(&payload)?;
            writer.flush()?;
            Ok(())
        }
        ["RESIZE", rows, cols] => {
            if let (Ok(rows), Ok(cols)) = (rows.parse::<u16>(), cols.parse::<u16>()) {
                master.resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })?;
            }
            Ok(())
        }
        ["KILL"] => {
            killer.lock().unwrap().kill()?;
            Ok(())
        }
        ["PING"] => Ok(()),
        ["OUTPUT_MODE"] => {
            let mode = output_mode(clients);
            writeln!(reader.get_mut(), "{mode}")?;
            reader.get_mut().flush()?;
            Ok(())
        }
        _ => bail!("unknown pty supervisor command: {}", line.trim()),
    }
}

fn remember(backlog: &Arc<Mutex<VecDeque<u8>>>, bytes: &[u8]) {
    let mut backlog = backlog.lock().unwrap();
    backlog.extend(bytes);
    while backlog.len() > BACKLOG_LIMIT {
        backlog.pop_front();
    }
}

pub(super) fn trace(message: &str) {
    let Ok(path) = std::env::var("MOSAICO_PTY_TRACE") else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{message}");
    }
}

pub(super) fn trace_bytes(label: &str, bytes: &[u8]) {
    let Ok(path) = std::env::var("MOSAICO_PTY_TRACE") else {
        return;
    };
    let hex = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{label}: {} bytes [{hex}]", bytes.len());
    }
}
