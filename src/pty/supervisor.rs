use anyhow::{bail, Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const BACKLOG_LIMIT: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct SupervisorArgs {
    pub id: String,
    pub socket: PathBuf,
    pub cwd: PathBuf,
    pub agent: String,
    pub channel: Option<String>,
    pub command: Vec<String>,
}

pub fn run_supervisor(args: SupervisorArgs) -> Result<()> {
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
    cmd.env("TENEX_EDGE_SPAWNED", "1");
    cmd.env("TENEX_EDGE_AGENT", &args.agent);
    cmd.env("TENEX_EDGE_PTY_SESSION", &args.id);
    cmd.env("TENEX_EDGE_PTY_SOCKET", args.socket.as_os_str());
    let term = std::env::var("TERM")
        .ok()
        .filter(|term| !term.is_empty() && term != "dumb")
        .unwrap_or_else(|| "xterm-256color".to_string());
    cmd.env("TERM", term);
    if let Some(channel) = &args.channel {
        cmd.env("TENEX_EDGE_CHANNEL", channel);
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
                    eprintln!("[tenex-edge pty supervisor] client error: {e:#}");
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

type Client = Arc<Mutex<UnixStream>>;
type Clients = Arc<Mutex<Vec<Client>>>;

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
        _ => bail!("unknown pty supervisor command: {}", line.trim()),
    }
}

fn attach_client(
    mut reader: BufReader<UnixStream>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    clients: &Clients,
    backlog: &Arc<Mutex<VecDeque<u8>>>,
) -> Result<()> {
    let mut output = reader.get_ref().try_clone()?;
    let remembered = backlog.lock().unwrap().iter().copied().collect::<Vec<_>>();
    if !remembered.is_empty() {
        let _ = output.write_all(&remembered);
    }
    clients.lock().unwrap().push(Arc::new(Mutex::new(output)));
    std::thread::spawn(move || {
        let mut buf = [0_u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    trace("supervisor attach eof");
                    break;
                }
                Ok(n) => {
                    trace_bytes("supervisor attach", &buf[..n]);
                    let mut writer = writer.lock().unwrap();
                    let result = writer.write_all(&buf[..n]).and_then(|_| writer.flush());
                    if result.is_err() {
                        trace("supervisor attach write error");
                        break;
                    }
                }
                Err(_) => {
                    trace("supervisor attach read error");
                    break;
                }
            }
        }
    });
    Ok(())
}

fn remember(backlog: &Arc<Mutex<VecDeque<u8>>>, bytes: &[u8]) {
    let mut backlog = backlog.lock().unwrap();
    backlog.extend(bytes);
    while backlog.len() > BACKLOG_LIMIT {
        backlog.pop_front();
    }
}

fn fanout(clients: &Clients, bytes: &[u8]) {
    let mut clients = clients.lock().unwrap();
    clients.retain(|client| {
        let Ok(mut stream) = client.lock() else {
            return false;
        };
        stream.write_all(bytes).and_then(|_| stream.flush()).is_ok()
    });
}

fn trace(message: &str) {
    let Ok(path) = std::env::var("TENEX_EDGE_PTY_TRACE") else {
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

fn trace_bytes(label: &str, bytes: &[u8]) {
    let Ok(path) = std::env::var("TENEX_EDGE_PTY_TRACE") else {
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
