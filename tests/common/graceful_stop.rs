//! Start external relays under a lifetime-pipe watcher and stop them so they
//! can close their store before a SIGKILL.
#![allow(dead_code)]

use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// Croissant bounds its HTTP drain at 5s and its store-close wait at 10s;
/// allow a margin on top.
const GRACE: Duration = Duration::from_secs(20);

/// The watcher runs `"$0" "$@"` with stdin from /dev/null and SIGTERMs it when
/// the shell's stdin (the lifetime pipe) closes: on an explicit stop, or when
/// the test process dies for any reason, terminal signal or not. The shell's
/// exit status is the relay's (`128 + N` if signal N killed it).
///
/// An asynchronous list's stdin is /dev/null in a non-interactive shell, so the
/// watcher reads the lifetime pipe through fd 3.
const WATCHER: &str = r#"exec 3<&0
"$0" "$@" </dev/null 3<&- & relay=$!
( read -r _ <&3; kill "$relay" 2>/dev/null ) &
exec 3<&-
wait "$relay""#;

/// A `/bin/sh` command that runs `program` under the lifetime watcher. Append
/// the program's arguments, environment and stdout/stderr to it, spawn it, and
/// take the lifetime pipe with `child.stdin.take()`. The shell leads a new
/// process group so a timed-out stop can SIGKILL the relay with it.
pub(crate) fn watched_command(program: &Path) -> Command {
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(WATCHER)
        .arg(program)
        .stdin(Stdio::piped())
        .process_group(0);
    command
}

/// Stop `child` and return how it exited, or `None` if it had to be SIGKILLed
/// or could not be reaped.
///
/// A watched child (`lifetime` is `Some`) is stopped by closing its lifetime
/// pipe; any other child gets SIGTERM. Either way it has [`GRACE`] to exit
/// before SIGKILL.
///
/// A SIGKILLed Croissant never closes LMDB. On macOS LMDB unlinks its POSIX
/// named semaphores only on a clean close, so every killed relay leaks them
/// until `kern.posix.sem.max` is exhausted and Croissant can no longer open
/// its store (mosaico #856).
pub(crate) fn stop_gracefully(
    child: &mut Child,
    lifetime: Option<ChildStdin>,
) -> Option<ExitStatus> {
    // Reap first: an exited, reaped child's pid may already belong to another
    // process. A failed poll is treated as "still running" so it gets stopped.
    if let Ok(Some(status)) = child.try_wait() {
        return Some(status);
    }
    let pid = nix::unistd::Pid::from_raw(child.id() as i32);
    let watched = lifetime.is_some();
    let signalled = match lifetime {
        Some(pipe) => {
            drop(pipe);
            true
        }
        None => nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM).is_ok(),
    };
    if signalled {
        let deadline = Instant::now() + GRACE;
        while Instant::now() < deadline {
            if let Ok(Some(status)) = child.try_wait() {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    if watched {
        // The shell leads the group, so this reaches the relay too.
        let _ = nix::sys::signal::killpg(pid, nix::sys::signal::Signal::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

/// On macOS, warn when a Croissant stop was not a clean exit: every unclean
/// stop leaks LMDB's POSIX semaphores. `watched` says whether `status` is the
/// watcher shell's (which reports a signal death as `128 + N`).
pub(crate) fn warn_unclean_croissant_stop(status: Option<ExitStatus>, watched: bool) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let Some(problem) = unclean_stop(status, watched) else {
        return;
    };
    eprintln!(
        "warning: Croissant {problem}, so it likely leaked LMDB POSIX semaphores \
         (mosaico #856). Check that $NIP29_RELAY_BIN is a current build that \
         closes its store on SIGTERM (pablof7z/croissant#13), not a stale or \
         older one; otherwise it crashed while closing. See its stderr log."
    );
}

fn unclean_stop(status: Option<ExitStatus>, watched: bool) -> Option<String> {
    let Some(status) = status else {
        return Some(format!(
            "did not exit within {}s of SIGTERM and was SIGKILLed",
            GRACE.as_secs()
        ));
    };
    if let Some(signal) = status.signal() {
        return Some(format!("was killed by signal {signal}"));
    }
    match status.code() {
        Some(0) | None => None,
        Some(code) if watched && code > 128 => Some(format!("was killed by signal {}", code - 128)),
        Some(code) => Some(format!("exited with status {code}")),
    }
}
