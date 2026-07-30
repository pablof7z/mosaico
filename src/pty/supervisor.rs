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
#[path = "supervisor/termination.rs"]
mod termination;

use clients::{attach_client, fanout, kill_if_headless, snapshot, Clients};

const BACKLOG_LIMIT: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct SupervisorArgs {
    pub id: String,
    pub instance_token: String,
    pub socket: PathBuf,
    pub cwd: PathBuf,
    pub agent: String,
    pub channel: Option<String>,
    pub session_name: Option<String>,
    pub ephemeral: bool,
    pub command: Vec<String>,
}

pub fn run_supervisor(args: SupervisorArgs) -> Result<()> {
    let clients = clients::new(args.id.clone());
    let backlog = Arc::new(Mutex::new(VecDeque::with_capacity(BACKLOG_LIMIT)));
    let mut session_exit_guard = session_exit::SessionExitGuard::new(
        args.id.clone(),
        clients.clone(),
        args.command.clone(),
        backlog.clone(),
    );
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
    cmd.env_remove("MOSAICO_CHANNEL");
    if let Some(session_name) = &args.session_name {
        cmd.env("MOSAICO_SESSION_NAME", session_name);
    } else {
        cmd.env_remove("MOSAICO_SESSION_NAME");
    }
    cmd.env_remove("CLAUDE_CODE_SESSION_ID");
    cmd.env_remove("CLAUDE_CODE_CHILD_SESSION");

    let mut child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);
    if let Err(error) =
        super::meta::record_child_pid(&args.id, &args.instance_token, child.process_id())
    {
        eprintln!("[mosaico pty supervisor] could not persist child identity: {error:#}");
    }
    let writer = Arc::new(Mutex::new(pair.master.take_writer()?));

    let mut output_pump = Some({
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
        })
    });

    // Binding is the public readiness boundary. Do it only after the harness
    // child exists and the output pump is installed; otherwise launchers can
    // attach to a socket whose supervisor is already failing startup.
    let _ = std::fs::remove_file(&args.socket);
    let listener = UnixListener::bind(&args.socket)
        .with_context(|| format!("binding {}", args.socket.display()))?;
    listener.set_nonblocking(true)?;

    loop {
        if let Some(status) = child.try_wait()? {
            session_exit_guard.record_child_exit(&args.agent, &status, &mut output_pump);
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                stream
                    .set_nonblocking(false)
                    .context("setting accepted pty client socket blocking")?;
                match handle_client(
                    stream,
                    pair.master.as_ref(),
                    &writer,
                    child.as_mut(),
                    &clients,
                    &backlog,
                ) {
                    Ok(Some((status, _presentation))) => {
                        session_exit_guard.record_child_exit(
                            &args.agent,
                            &status,
                            &mut output_pump,
                        );
                        break;
                    }
                    Ok(None) => {}
                    Err(e) => eprintln!("[mosaico pty supervisor] client error: {e:#}"),
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
    child: &mut (dyn portable_pty::Child + Send + Sync),
    clients: &Clients,
    backlog: &Arc<Mutex<VecDeque<u8>>>,
) -> Result<Option<(portable_pty::ExitStatus, crate::pty::PresentationSnapshot)>> {
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
            attach_client(reader, writer.clone(), clients, backlog)?;
            Ok(None)
        }
        ["INJECT", len] => {
            let len = len.parse::<usize>().context("invalid INJECT length")?;
            let mut payload = vec![0_u8; len];
            reader.read_exact(&mut payload)?;
            trace_bytes("supervisor inject", &payload);
            let mut writer = writer.lock().unwrap();
            writer.write_all(&payload)?;
            writer.flush()?;
            Ok(None)
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
            Ok(None)
        }
        ["KILL"] => {
            let status = termination::terminate_child_confirmed(child)?;
            Ok(Some((status, snapshot(clients))))
        }
        ["PING"] => Ok(None),
        ["STARTED"] => {
            if let Some(status) = child.try_wait()? {
                return Ok(Some((status, snapshot(clients))));
            }
            reader.get_mut().write_all(b"READY\n")?;
            reader.get_mut().flush()?;
            Ok(None)
        }
        ["PRESENTATION"] => {
            let presentation = snapshot(clients);
            serde_json::to_writer(reader.get_mut(), &presentation)?;
            reader.get_mut().write_all(b"\n")?;
            reader.get_mut().flush()?;
            Ok(None)
        }
        ["KILL_IF_HEADLESS", expected_epoch] => {
            let expected_epoch = expected_epoch
                .parse::<u64>()
                .context("invalid attachment epoch")?;
            let mut confirmed_status = None;
            let outcome = kill_if_headless(clients, expected_epoch, |_| {
                confirmed_status = Some(termination::terminate_child_confirmed(child)?);
                Ok(())
            })?;
            serde_json::to_writer(reader.get_mut(), &outcome)?;
            reader.get_mut().write_all(b"\n")?;
            reader.get_mut().flush()?;
            Ok(confirmed_status.map(|status| (status, snapshot(clients))))
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
