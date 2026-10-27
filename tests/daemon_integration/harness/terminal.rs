use super::*;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::Read as _;
use std::sync::{Arc, Mutex};

/// Printed by the CLI once an interactive launch has handed its PTY to the
/// caller's terminal (`Launched <handle>`), or a resume has attached to one.
const HANDOFF_MARKERS: [&str; 3] = ["Launched ", "Adopted and resumed @", "Attached to @"];

/// What the user saw in their terminal, and how the CLI ended.
pub(crate) struct TerminalRun {
    /// Combined stdout/stderr as rendered on the terminal.
    pub(crate) output: String,
    /// Whether the CLI reached its terminal hand-off before it was detached.
    pub(crate) handed_off: bool,
    /// The CLI's own exit status when it ended by itself; `None` when the test
    /// detached it after the hand-off, like closing the terminal window.
    pub(crate) exit: Option<portable_pty::ExitStatus>,
}

impl TerminalRun {
    /// The CLI refused or failed the launch: it exited unsuccessfully before
    /// handing off a terminal.
    pub(crate) fn failed(&self) -> bool {
        !self.handed_off && self.exit.as_ref().is_some_and(|status| !status.success())
    }
}

/// Run the real CLI with its stdin and stdout on a pseudo-terminal: the
/// interactive caller a person has. Launch intent follows the caller's
/// terminal, so only this path selects the interactive PTY transport.
///
/// Returns once the CLI either exits on its own or reaches its hand-off and
/// attaches; an attached CLI is then detached with SIGHUP, which is what a
/// closing terminal sends. The hosted PTY keeps running headless.
pub(crate) fn run_cli_in_terminal(
    home: &Home,
    args: &[&str],
    env: &[(&str, &str)],
    cwd: &std::path::Path,
) -> TerminalRun {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open test terminal");
    let mut command = CommandBuilder::new(bin());
    command.args(args);
    command.cwd(cwd);
    for key in [
        "MOSAICO_AGENT",
        "MOSAICO_PUBKEY",
        "MOSAICO_PTY_SESSION",
        "MOSAICO_PTY_SOCKET",
        "MOSAICO_CHANNEL",
        "MOSAICO_EPHEMERAL",
        "MOSAICO",
    ] {
        command.env_remove(key);
    }
    command.env("MOSAICO_HOME", home.dir.path());
    command.env("MOSAICO_CONFIG", home.dir.path().join("config.json"));
    command.env("MOSAICO_BIN", bin());
    command.env("MOSAICO_DAEMON_GRACE_S", "30");
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = pair.slave.spawn_command(command).expect("spawn CLI");
    drop(pair.slave);

    let output = Arc::new(Mutex::new(String::new()));
    let mut reader = pair.master.try_clone_reader().expect("terminal reader");
    let pump = {
        let output = output.clone();
        std::thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                output
                    .lock()
                    .unwrap()
                    .push_str(&String::from_utf8_lossy(&buffer[..read]));
            }
        })
    };
    let rendered = || output.lock().unwrap().clone();

    let mut handed_off = false;
    let mut exit = None;
    assert!(
        wait_until(Duration::from_secs(60), || {
            if let Some(status) = child.try_wait().expect("poll CLI") {
                exit = Some(status);
                return true;
            }
            handed_off = HANDOFF_MARKERS
                .iter()
                .any(|marker| rendered().contains(marker));
            handed_off
        }),
        "CLI neither exited nor handed off its terminal: {:?}",
        rendered()
    );
    if exit.is_none() {
        let pid = child.process_id().expect("CLI pid");
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(i32::try_from(pid).expect("CLI pid")),
            nix::sys::signal::Signal::SIGHUP,
        )
        .expect("detach CLI from its terminal");
        assert!(
            wait_until(Duration::from_secs(10), || child
                .try_wait()
                .expect("poll CLI")
                .is_some()),
            "CLI ignored terminal hang-up: {:?}",
            rendered()
        );
    }
    drop(pair.master);
    pump.join().expect("terminal output pump");
    TerminalRun {
        output: rendered(),
        handed_off,
        exit,
    }
}
