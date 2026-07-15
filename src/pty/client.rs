use super::meta;
use anyhow::{bail, Context, Result};
use crossterm::terminal;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::Duration;

#[path = "client/lifecycle.rs"]
mod lifecycle;
#[path = "client/terminal_mode.rs"]
mod terminal_mode;

use terminal_mode::TerminalMode;

pub struct AttachStream {
    id_or_path: String,
    stream: UnixStream,
}

pub fn attach_stream(id_or_path: &str, rows: u16, cols: u16) -> Result<AttachStream> {
    AttachStream::connect(id_or_path, rows, cols)
}

impl AttachStream {
    fn connect(id_or_path: &str, rows: u16, cols: u16) -> Result<Self> {
        let mut stream = connect(id_or_path)?;
        writeln!(stream, "ATTACH {rows} {cols}")?;
        stream.flush()?;
        stream
            .set_nonblocking(true)
            .context("setting pty attach stream nonblocking")?;
        Ok(Self {
            id_or_path: id_or_path.to_string(),
            stream,
        })
    }

    pub fn read_available(&mut self, out: &mut Vec<u8>) -> Result<bool> {
        let mut buf = [0_u8; 8192];
        loop {
            match self.stream.read(&mut buf) {
                Ok(0) => return Ok(false),
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(true),
                Err(e) if is_disconnect(e.kind()) => return Ok(false),
                Err(e) => return Err(e).context("reading pty attach stream"),
            }
        }
    }

    pub fn write_input(&mut self, bytes: &[u8]) -> Result<bool> {
        trace_bytes("client tui", bytes);
        match self
            .stream
            .write_all(bytes)
            .and_then(|_| self.stream.flush())
        {
            Ok(()) => Ok(true),
            Err(e) if is_disconnect(e.kind()) => Ok(false),
            Err(e) => Err(e).context("writing pty attach stream"),
        }
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        resize(&self.id_or_path, rows, cols)
    }

    pub fn shutdown(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

pub fn list() -> Result<()> {
    let rows = meta::read_all_metadata();
    if rows.is_empty() {
        println!("No portable-pty sessions found.");
        return Ok(());
    }
    println!("{:<28} {:<10} {:<5} command", "id", "agent", "live");
    for row in rows {
        let live = if is_live(&row.id) { "yes" } else { "no" };
        println!(
            "{:<28} {:<10} {:<5} {}",
            row.id,
            row.agent,
            live,
            row.command.join(" ")
        );
    }
    Ok(())
}

pub fn is_live(id_or_path: &str) -> bool {
    let path = meta::resolve_socket(id_or_path);
    UnixStream::connect(&path)
        .and_then(|mut stream| stream.write_all(b"PING\n"))
        .is_ok()
}

pub fn inject(id_or_path: &str, text: &str, bracketed: bool, submit: bool) -> Result<()> {
    let mut payload = Vec::new();
    if bracketed {
        payload.extend_from_slice(b"\x1b[200~");
    }
    payload.extend_from_slice(text.as_bytes());
    if bracketed {
        payload.extend_from_slice(b"\x1b[201~");
    }
    send_inject(id_or_path, &payload)?;
    if submit {
        std::thread::sleep(Duration::from_millis(30));
        send_inject(id_or_path, b"\r")?;
    }
    Ok(())
}

fn send_inject(id_or_path: &str, payload: &[u8]) -> Result<()> {
    let mut stream = connect(id_or_path)?;
    writeln!(stream, "INJECT {}", payload.len())?;
    stream.write_all(payload)?;
    stream.flush()?;
    Ok(())
}

pub fn kill(id_or_path: &str) -> Result<()> {
    let mut stream = connect(id_or_path)?;
    stream.write_all(b"KILL\n")?;
    Ok(())
}

pub fn resize(id_or_path: &str, rows: u16, cols: u16) -> Result<()> {
    let mut stream = connect(id_or_path)?;
    writeln!(stream, "RESIZE {rows} {cols}")?;
    stream.flush()?;
    Ok(())
}

pub fn attach(id_or_path: &str, agent_handle: &str) -> Result<()> {
    let mut stream = connect(id_or_path)?;
    let (cols, rows) = terminal::size().unwrap_or((80, 24));
    writeln!(stream, "ATTACH {rows} {cols}")?;

    let terminal = TerminalMode::enter()?;
    let mut stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut input = [0_u8; 1024];
    let mut output = [0_u8; 8192];
    let mut attach_error = None;
    let mut last_size = (cols, rows);
    let detach_reason = loop {
        let mut fds = [
            libc::pollfd {
                fd: stdin.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: stream.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 250) };
        if rc < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == ErrorKind::Interrupted {
                continue;
            }
            attach_error = Some(anyhow::anyhow!("polling terminal and pty: {e}"));
            break "poll error";
        }
        if let Some(size) = current_size_if_changed(last_size) {
            last_size = size;
            let (cols, rows) = size;
            if let Err(e) = resize(id_or_path, rows, cols) {
                attach_error = Some(anyhow::anyhow!("resizing pty: {e}"));
                break "resize error";
            }
        }
        if rc == 0 {
            continue;
        }

        if has_event(fds[1].revents, libc::POLLIN) {
            match stream.read(&mut output) {
                Ok(0) => break "pty output closed",
                Ok(n) => {
                    if let Err(e) = stdout.write_all(&output[..n]).and_then(|_| stdout.flush()) {
                        attach_error = Some(anyhow::anyhow!("writing pty output to terminal: {e}"));
                        break "terminal output error";
                    }
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) if is_disconnect(e.kind()) => break "pty disconnected",
                Err(e) => {
                    attach_error = Some(anyhow::anyhow!("reading pty output: {e}"));
                    break "pty output error";
                }
            }
        } else if has_hup_or_error(fds[1].revents) {
            break "pty output closed";
        }

        if has_event(fds[0].revents, libc::POLLIN) {
            match stdin.read(&mut input) {
                Ok(0) => break "stdin eof",
                Ok(n) => {
                    trace_bytes("client stdin", &input[..n]);
                    if let Err(e) = stream.write_all(&input[..n]).and_then(|_| stream.flush()) {
                        if is_disconnect(e.kind()) {
                            break "pty disconnected";
                        }
                        attach_error = Some(anyhow::anyhow!("writing terminal input to pty: {e}"));
                        break "pty input error";
                    }
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) if is_disconnect(e.kind()) => break "stdin disconnected",
                Err(e) => {
                    attach_error = Some(anyhow::anyhow!("reading terminal input: {e}"));
                    break "stdin error";
                }
            }
        } else if has_hup_or_error(fds[0].revents) {
            break "stdin closed";
        }
    };
    let _ = stream.shutdown(std::net::Shutdown::Both);
    drop(terminal);
    eprintln!("\r\n{}", lifecycle::message(agent_handle, detach_reason));
    attach_error.map_or(Ok(()), Err)
}

fn current_size_if_changed(last_size: (u16, u16)) -> Option<(u16, u16)> {
    let size = terminal::size().ok()?;
    (size != last_size).then_some(size)
}

fn has_event(revents: i16, flag: i16) -> bool {
    revents & flag != 0
}

fn has_hup_or_error(revents: i16) -> bool {
    revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0
}

fn is_disconnect(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::BrokenPipe | ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted
    )
}

fn trace_bytes(label: &str, bytes: &[u8]) {
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

fn connect(id_or_path: &str) -> Result<UnixStream> {
    let path = meta::resolve_socket(id_or_path);
    for _ in 0..50 {
        match UnixStream::connect(&path) {
            Ok(s) => return Ok(s),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    bail!("could not connect to pty socket {}", path.display())
}
